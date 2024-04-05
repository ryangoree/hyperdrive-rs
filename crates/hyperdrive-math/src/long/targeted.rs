use ethers::types::I256;
use eyre::{eyre, Result};
use fixed_point::FixedPoint;
use fixed_point_macros::fixed;

use crate::{State, YieldSpace};

impl State {
    /// Gets a target long that can be opened given a budget to achieve a desired fixed rate.
    ///
    /// If the long amount to reach the target is greater than the budget, the budget is returned.
    /// If the long amount to reach the target is invalid (i.e. it would produce an insolvent pool), then
    /// an error is thrown, and the user is advised to use [calculate_max_long](long::max::calculate_max_long).
    pub fn calculate_targeted_long_with_budget<
        F1: Into<FixedPoint>,
        F2: Into<FixedPoint>,
        F3: Into<FixedPoint>,
        I: Into<I256>,
    >(
        &self,
        budget: F1,
        target_rate: F2,
        checkpoint_exposure: I,
        maybe_max_iterations: Option<usize>,
        maybe_allowable_error: Option<F3>,
    ) -> Result<FixedPoint> {
        let budget = budget.into();
        match self.calculate_targeted_long(
            target_rate,
            checkpoint_exposure,
            maybe_max_iterations,
            maybe_allowable_error,
        ) {
            Ok(long_amount) => Ok(long_amount.min(budget)),
            Err(error) => Err(error),
        }
    }

    /// Gets a target long that can be opened to achieve a desired fixed rate.
    fn calculate_targeted_long<F1: Into<FixedPoint>, F2: Into<FixedPoint>, I: Into<I256>>(
        &self,
        target_rate: F1,
        checkpoint_exposure: I,
        maybe_max_iterations: Option<usize>,
        maybe_allowable_error: Option<F2>,
    ) -> Result<FixedPoint> {
        let target_rate = target_rate.into();
        let checkpoint_exposure = checkpoint_exposure.into();
        let allowable_error = match maybe_allowable_error {
            Some(allowable_error) => allowable_error.into(),
            None => fixed!(1e14),
        };

        // Estimate the long that achieves a target rate.
        let (target_share_reserves, target_bond_reserves) =
            self.reserves_given_rate_ignoring_exposure(target_rate);
        let (target_base_delta, target_bond_delta) =
            self.trade_deltas_from_reserves(target_share_reserves, target_bond_reserves);

        // Determine what rate was achieved.
        let resulting_rate = self.rate_after_long(target_base_delta, Some(target_bond_delta))?;

        // The estimated long should always underestimate because the realized price
        // should always be greater than the spot price.
        if target_rate > resulting_rate {
            return Err(eyre!("get_targeted_long: We overshot the zero-crossing.",));
        }
        let rate_error = resulting_rate - target_rate;

        // If solvent & within the allowable error, stop here.
        if self
            .solvency_after_long(target_base_delta, target_bond_delta, checkpoint_exposure)
            .is_some()
            && rate_error < allowable_error
        {
            Ok(target_base_delta)
        }
        // Else, iterate to find a solution.
        else {
            // We can use the initial guess as a starting point since we know it is less than the target.
            let mut possible_target_base_delta = target_base_delta;

            // Iteratively find a solution
            for _ in 0..maybe_max_iterations.unwrap_or(7) {
                let possible_target_bond_delta = self
                    .calculate_open_long(possible_target_base_delta)
                    .unwrap();
                let resulting_rate = self.rate_after_long(
                    possible_target_base_delta,
                    Some(possible_target_bond_delta),
                )?;

                // We assume that the loss is positive only because Newton's
                // method and the one-shot approximation will always underestimate.
                if target_rate > resulting_rate {
                    return Err(eyre!("get_targeted_long: We overshot the zero-crossing.",));
                }
                // The loss is $l(x) = r(x) - r_t$ for some rate after a long
                // is opened, $r(x)$, and target rate, $r_t$.
                let loss = resulting_rate - target_rate;

                // If we've done it (solvent & within error), then return the value.
                if self
                    .solvency_after_long(
                        possible_target_base_delta,
                        possible_target_bond_delta,
                        checkpoint_exposure,
                    )
                    .is_some()
                    && loss < allowable_error
                {
                    return Ok(possible_target_base_delta);
                }
                // Otherwise perform another iteration.
                else {
                    // The derivative of the loss is $l'(x) = r'(x)$.
                    // We return $-l'(x)$ because $r'(x)$ is negative, which
                    // can't be represented with FixedPoint.
                    let negative_loss_derivative = self.rate_after_long_derivative_negation(
                        possible_target_base_delta,
                        possible_target_bond_delta,
                    )?;

                    // Adding the negative loss derivative instead of subtracting the loss derivative
                    // ∆x_{n+1} = ∆x_{n} - l / l'
                    //          = ∆x_{n} + l / (-l')
                    possible_target_base_delta =
                        possible_target_base_delta + loss / negative_loss_derivative;
                }
            }

            // Final solvency check.
            if self
                .solvency_after_long(
                    possible_target_base_delta,
                    self.calculate_open_long(possible_target_base_delta)
                        .unwrap(),
                    checkpoint_exposure,
                )
                .is_none()
            {
                return Err(eyre!("Guess in `get_targeted_long` is insolvent."));
            }

            // Final accuracy check.
            let possible_target_bond_delta = self
                .calculate_open_long(possible_target_base_delta)
                .unwrap();
            let resulting_rate =
                self.rate_after_long(possible_target_base_delta, Some(possible_target_bond_delta))?;
            if target_rate > resulting_rate {
                return Err(eyre!("get_targeted_long: We overshot the zero-crossing.",));
            }
            let loss = resulting_rate - target_rate;
            if loss >= allowable_error {
                return Err(eyre!(
                    "get_targeted_long: Unable to find an acceptable loss. Final loss = {}.",
                    loss
                ));
            }

            Ok(possible_target_base_delta)
        }
    }

    /// The fixed rate after a long has been opened.
    ///
    /// We calculate the rate for a fixed length of time as:
    /// $$
    /// r(x) = (1 - p(x)) / (p(x) t)
    /// $$
    ///
    /// where $p(x)$ is the spot price after a long for `delta_bonds`$= x$ and
    /// t is the normalized position druation.
    ///
    /// In this case, we use the resulting spot price after a hypothetical long
    /// for `base_amount` is opened.
    fn rate_after_long(
        &self,
        base_amount: FixedPoint,
        bond_amount: Option<FixedPoint>,
    ) -> Result<FixedPoint> {
        let resulting_price = self.calculate_spot_price_after_long(base_amount, bond_amount)?;
        Ok((fixed!(1e18) - resulting_price)
            / (resulting_price * self.annualized_position_duration()))
    }

    /// The derivative of the equation for calculating the rate after a long.
    ///
    /// For some $r = (1 - p(x)) / (p(x) \cdot t)$, where $p(x)$
    /// is the spot price after a long of `delta_base`$= x$ was opened and $t$
    /// is the annualized position duration, the rate derivative is:
    ///
    /// $$
    /// r'(x) = \frac{(-p'(x) \cdot p(x) t - (1 - p(x)) (p'(x) \cdot t))}{(p(x) \cdot t)^2} //
    /// r'(x) = \frac{-p'(x)}{t \cdot p(x)^2}
    /// $$
    ///
    /// We return $-r'(x)$ because negative numbers cannot be represented by FixedPoint.
    fn rate_after_long_derivative_negation(
        &self,
        base_amount: FixedPoint,
        bond_amount: FixedPoint,
    ) -> Result<FixedPoint> {
        let price = self.calculate_spot_price_after_long(base_amount, Some(bond_amount))?;
        let price_derivative = self.price_after_long_derivative(base_amount, bond_amount)?;
        // The actual equation we want to represent is:
        // r' = -p' / (t \cdot p^2)
        // We can do a trick to return a positive-only version and
        // indicate that it should be negative in the fn name.
        // We use price * price instead of price.pow(fixed!(2e18)) to avoid error introduced by pow.
        Ok(price_derivative / (self.annualized_position_duration() * price * price))
    }

    /// The derivative of the price after a long.
    ///
    /// The price after a long that moves shares by $\Delta z$ and bonds by $\Delta y$
    /// is equal to
    ///
    /// $$
    /// p(\Delta z) = (\frac{\mu \cdot (z_{0} + \Delta z - (\zeta_{0} + \Delta \zeta))}{y - \Delta y})^{t_{s}}
    /// $$
    ///
    /// where $t_{s}$ is the time stretch constant and $z_{e,0}$ is the initial
    /// effective share reserves, and $\zeta$ is the zeta adjustment.
    /// The zeta adjustment is constant when opening a long, i.e.
    /// $\Delta \zeta = 0$, so we drop the subscript. Equivalently, for some
    /// amount of `delta_base`$= x$ provided to open a long, we can write:
    ///
    /// $$
    /// p(x) = (\frac{\mu \cdot (z_{e,0} + \frac{x}{c} - g(x) - \zeta)}{y_0 - y(x)})^{t_{s}}
    /// $$
    ///
    /// where $g(x)$ is the [open_long_governance_fee](long::fees::open_long_governance_fee),
    /// $y(x)$ is the [long_amount](long::open::calculate_open_long),
    ///
    ///
    /// To compute the derivative, we first define some auxiliary variables:
    ///
    /// $$
    /// a(x) = \mu (z_{0} + \frac{x}{c} - g(x) - \zeta) \\
    /// b(x) = y_0 - y(x) \\
    /// v(x) = \frac{a(x)}{b(x)}
    /// $$
    ///
    /// and thus $p(x) = v(x)^t_{s}$. Given these, we can write out intermediate derivatives:
    ///
    /// $$
    /// a'(x) = \frac{\mu}{c} - g'(x) \\
    /// b'(x) = -y'(x) \\
    /// v'(x) = \frac{b(x) \cdot a'(x) - a(x) \cdot b'(x)}{b(x)^2}
    /// $$
    ///
    /// And finally, the price after long derivative is:
    ///
    /// $$
    /// p'(x) = v'(x) \cdot t_{s} \cdot v(x)^(t_{s} - 1)
    /// $$
    ///
    fn price_after_long_derivative(
        &self,
        base_amount: FixedPoint,
        bond_amount: FixedPoint,
    ) -> Result<FixedPoint> {
        // g'(x)
        let gov_fee_derivative = self.governance_lp_fee()
            * self.curve_fee()
            * (fixed!(1e18) - self.calculate_spot_price());

        // a(x) = mu * (z_{e,0} + x/c - g(x))
        let inner_numerator = self.mu()
            * (self.ze() + base_amount / self.vault_share_price()
                - self.open_long_governance_fee(base_amount));

        // a'(x) = mu / c - g'(x)
        let inner_numerator_derivative = self.mu() / self.vault_share_price() - gov_fee_derivative;

        // b(x) = y_0 - y(x)
        let inner_denominator = self.bond_reserves() - bond_amount;

        // b'(x) = -y'(x)
        let long_amount_derivative = match self.long_amount_derivative(base_amount) {
            Some(derivative) => derivative,
            None => return Err(eyre!("long_amount_derivative failure.")),
        };

        // v(x) = a(x) / b(x)
        // v'(x) = ( b(x) * a'(x) - a(x) * b'(x) ) / b(x)^2
        //       = ( b(x) * a'(x) + a(x) * -b'(x) ) / b(x)^2
        // Note that we are adding the negative b'(x) to avoid negative fixedpoint numbers
        let inner_derivative = (inner_denominator * inner_numerator_derivative
            + inner_numerator * long_amount_derivative)
            / (inner_denominator * inner_denominator);

        // p'(x) = v'(x) * t_s * v(x)^(t_s - 1)
        // p'(x) = v'(x) * t_s * v(x)^(-1)^(1 - t_s)
        // v(x) is flipped to (denominator / numerator) to avoid a negative exponent
        Ok(inner_derivative
            * self.time_stretch()
            * (inner_denominator / inner_numerator).pow(fixed!(1e18) - self.time_stretch()))
    }

    /// Calculate the base & bond deltas from the current state given desired new reserve levels.
    ///
    /// Given a target ending pool share reserves, $z_t$, and bond reserves, $y_t$,
    /// the trade deltas to achieve that state would be:
    ///
    /// $$
    /// \Delta x = c \cdot (z_t - z_{e,0}) \\
    /// \Delta y = y - y_t - c(\Delta x)
    /// $$
    ///
    /// where $c$ is the vault share price and
    /// $c(\Delta x)$ is the (open_long_curve_fee)[long::fees::open_long_curve_fees].
    fn trade_deltas_from_reserves(
        &self,
        share_reserves: FixedPoint,
        bond_reserves: FixedPoint,
    ) -> (FixedPoint, FixedPoint) {
        let base_delta =
            (share_reserves - self.effective_share_reserves()) * self.vault_share_price();
        let bond_delta =
            (self.bond_reserves() - bond_reserves) - self.open_long_curve_fees(base_delta);
        (base_delta, bond_delta)
    }

    /// Calculates the pool reserve levels to achieve a target interest rate.
    /// This calculation does not take Hyperdrive's solvency constraints or exposure
    /// into account and shouldn't be used directly.
    ///
    /// The price for a given fixed-rate is given by $p = 1 / (r \cdot t + 1)$, where
    /// $r$ is the fixed-rate and $t$ is the annualized position duration. The
    /// price for a given pool reserves is given by $p = \frac{\mu z}{y}^t_{s}$,
    /// where $\mu$ is the initial share price and $t_{s}$ is the time stretch
    /// constant. By setting these equal we can solve for the pool reserve levels
    /// as a function of a target rate.
    ///
    /// For some target rate, $r_t$, the pool share reserves, $z_t$, must be:
    ///
    /// $$
    /// z_t = \frac{1}{\mu} \left(
    ///   \frac{k}{\frac{c}{\mu} + \left(
    ///     (r_t \cdot t + 1)^{\frac{1}{t_{s}}}
    ///   \right)^{1 - t_{s}}}
    /// \right)^{\tfrac{1}{1 - t_{s}}}
    /// $$
    ///
    /// and the pool bond reserves, $y_t$, must be:
    ///
    /// $$
    /// y_t = \left(
    ///   \frac{k}{ \frac{c}{\mu} +  \left(
    ///     \left( r_t \cdot t + 1 \right)^{\frac{1}{t_{s}}}
    ///   \right)^{1 - t_{s}}}
    /// \right)^{1 - t_{s}} \left( r_t t + 1 \right)^{\frac{1}{t_{s}}}
    /// $$
    fn reserves_given_rate_ignoring_exposure<F: Into<FixedPoint>>(
        &self,
        target_rate: F,
    ) -> (FixedPoint, FixedPoint) {
        let target_rate = target_rate.into();

        // First get the target share reserves
        let c_over_mu = self
            .vault_share_price()
            .div_up(self.initial_vault_share_price());
        let scaled_rate = (target_rate.mul_up(self.annualized_position_duration()) + fixed!(1e18))
            .pow(fixed!(1e18) / self.time_stretch());
        let inner = (self.k_down()
            / (c_over_mu + scaled_rate.pow(fixed!(1e18) - self.time_stretch())))
        .pow(fixed!(1e18) / (fixed!(1e18) - self.time_stretch()));
        let target_share_reserves = inner / self.initial_vault_share_price();

        // Then get the target bond reserves.
        let target_bond_reserves = inner * scaled_rate;

        (target_share_reserves, target_bond_reserves)
    }
}

#[cfg(test)]
mod tests {
    use std::panic;

    use ethers::types::U256;
    use rand::{thread_rng, Rng};
    use test_utils::{
        agent::Agent,
        chain::{Chain, TestChain},
        constants::FUZZ_RUNS,
    };
    use tracing_test::traced_test;

    use super::*;

    #[traced_test]
    #[tokio::test]
    async fn test_calculate_targeted_long_with_budget() -> Result<()> {
        // Spawn a test chain and create two agents -- Alice and Bob.
        // Alice is funded with a large amount of capital so that she can initialize
        // the pool. Bob is funded with a random amount of capital so that we
        // can test `calculate_targeted_long` when budget is the primary constraint
        // and when it is not.

        let allowable_solvency_error = fixed!(1e5);
        let allowable_budget_error = fixed!(1e5);
        let allowable_rate_error = fixed!(1e10);
        let num_newton_iters = 5;

        // Initialize a test chain. We don't need mocks because we want state updates.
        let chain = TestChain::new(2).await?;

        // Grab accounts for Alice and Bob.
        let (alice, bob) = (chain.accounts()[0].clone(), chain.accounts()[1].clone());

        // Initialize Alice and Bob as Agents.
        let mut alice =
            Agent::new(chain.client(alice).await?, chain.addresses().clone(), None).await?;
        let mut bob = Agent::new(chain.client(bob).await?, chain.addresses(), None).await?;
        let config = bob.get_config().clone();

        // Fuzz test
        let mut rng = thread_rng();
        for _ in 0..*FUZZ_RUNS {
            // Snapshot the chain.
            let id = chain.snapshot().await?;

            // Fund Alice and Bob.
            // Large budget for initializing the pool.
            let contribution = fixed!(1_000_000e18);
            alice.fund(contribution).await?;
            // Small lower bound on the budget for resource-constrained targeted longs.
            let budget = rng.gen_range(fixed!(10e18)..=fixed!(500_000_000e18));

            // Alice initializes the pool.
            let initial_fixed_rate = rng.gen_range(fixed!(0.01e18)..=fixed!(0.1e18));
            alice
                .initialize(initial_fixed_rate, contribution, None)
                .await?;

            // Half the time we will open a long & let it mature.
            if rng.gen_range(0..=1) == 0 {
                // Open a long.
                let max_long =
                    bob.get_state()
                        .await?
                        .calculate_max_long(U256::MAX, I256::from(0), None);
                let long_amount =
                    (max_long / fixed!(100e18)).max(config.minimum_transaction_amount.into());
                bob.fund(long_amount + budget).await?;
                bob.open_long(long_amount, None, None).await?;
                // Advance time to just after maturity.
                let variable_rate = rng.gen_range(fixed!(0)..=fixed!(0.5e18));
                let time_amount = FixedPoint::from(config.position_duration) * fixed!(105e17); // 1.05 * position_duraiton
                alice.advance_time(variable_rate, time_amount).await?;
                // Checkpoint to auto-close the position.
                alice
                    .checkpoint(alice.latest_checkpoint().await?, None)
                    .await?;
            }
            // Else we will just fund a random budget amount and do the targeted long.
            else {
                bob.fund(budget).await?;
            }

            // Some of the checkpoint passes and variable interest accrues.
            alice
                .checkpoint(alice.latest_checkpoint().await?, None)
                .await?;
            let variable_rate = rng.gen_range(fixed!(0)..=fixed!(0.5e18));
            alice
                .advance_time(
                    variable_rate,
                    FixedPoint::from(config.checkpoint_duration) * fixed!(0.5e18),
                )
                .await?;

            // Bob opens a targeted long.
            let max_spot_price_before_long = bob.get_state().await?.calculate_max_spot_price();
            let target_rate = initial_fixed_rate / fixed!(2e18);
            let targeted_long = bob
                .calculate_targeted_long(
                    target_rate,
                    Some(num_newton_iters),
                    Some(allowable_rate_error),
                )
                .await?;
            bob.open_long(targeted_long, None, None).await?;

            // Three things should be true after opening the long:
            //
            // 1. The pool's spot price is under the max spot price prior to
            //    considering fees
            // 2. The pool's solvency is above zero.
            // 3. IF Bob's budget is not consumed; then new rate is close to the target rate

            // Check that our resulting price is under the max
            let spot_price_after_long = bob.get_state().await?.calculate_spot_price();
            assert!(
                max_spot_price_before_long > spot_price_after_long,
                "Resulting price is greater than the max."
            );

            // Check solvency
            let is_solvent =
                { bob.get_state().await?.calculate_solvency() > allowable_solvency_error };
            assert!(is_solvent, "Resulting pool state is not solvent.");

            let new_rate = bob.get_state().await?.calculate_spot_rate();
            // If the budget was NOT consumed, then we assume the target was hit.
            if !(bob.base() <= allowable_budget_error) {
                // Actual price might result in long overshooting the target.
                let abs_error = if target_rate > new_rate {
                    target_rate - new_rate
                } else {
                    new_rate - target_rate
                };
                assert!(
                    abs_error <= allowable_rate_error,
                    "target_rate was {}, realized rate is {}. abs_error={} was not <= {}.",
                    target_rate,
                    new_rate,
                    abs_error,
                    allowable_rate_error
                );
            }
            // Else, we should have undershot,
            // or by some coincidence the budget was the perfect amount
            // and we hit the rate exactly.
            else {
                assert!(
                    new_rate <= target_rate,
                    "The new_rate={} should be <= target_rate={} when budget constrained.",
                    new_rate,
                    target_rate
                );
            }

            // Revert to the snapshot and reset the agent's wallets.
            chain.revert(id).await?;
            alice.reset(Default::default());
            bob.reset(Default::default());
        }

        Ok(())
    }
}

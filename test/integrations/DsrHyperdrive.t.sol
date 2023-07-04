// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.19;

import { IERC20 } from "contracts/src/interfaces/IERC20.sol";
import { IHyperdrive } from "contracts/src/interfaces/IHyperdrive.sol";
import { FixedPointMath } from "contracts/src/libraries/FixedPointMath.sol";
import { ForwarderFactory } from "contracts/src/token/ForwarderFactory.sol";
import { IMockDsrHyperdrive, MockDsrHyperdrive, MockDsrHyperdriveDataProvider, DsrManager } from "contracts/test/MockDsrHyperdrive.sol";
import { BaseTest } from "test/utils/BaseTest.sol";
import { HyperdriveMath } from "contracts/src/libraries/HyperdriveMath.sol";

contract DsrHyperdrive is BaseTest {
    using FixedPointMath for uint256;

    IMockDsrHyperdrive hyperdrive;
    IERC20 dai;
    IERC20 chai;
    DsrManager dsrManager;

    function setUp() public override __mainnet_fork(16_685_972) {
        super.setUp();

        dai = IERC20(address(0x6B175474E89094C44Da98b954EedeAC495271d0F));
        dsrManager = DsrManager(
            address(0x373238337Bfe1146fb49989fc222523f83081dDb)
        );

        vm.startPrank(deployer);
        address dataProvider = address(
            new MockDsrHyperdriveDataProvider(dsrManager)
        );
        hyperdrive = IMockDsrHyperdrive(
            address(new MockDsrHyperdrive(dataProvider, dsrManager))
        );

        address daiWhale = 0x075e72a5eDf65F0A5f44699c7654C1a76941Ddc8;

        whaleTransfer(daiWhale, dai, alice);

        vm.stopPrank();
        vm.startPrank(alice);
        dai.approve(address(hyperdrive), type(uint256).max);

        vm.stopPrank();
        vm.startPrank(bob);
        dai.approve(address(hyperdrive), type(uint256).max);
    }

    function test__base_token_is_dai() public {
        assertEq(
            address(hyperdrive.baseToken()),
            address(dai),
            "constructor call to dsrManager.dai() returned an invalid dai contract"
        );
    }

    function test__dai_token_is_approved() public {
        uint256 allowance = dai.allowance(
            address(hyperdrive),
            address(dsrManager)
        );
        assertEq(
            allowance,
            type(uint256).max,
            "dsrManager should be an approved DAI spender of hyperdrive"
        );
    }

    function test__initial_base_token_deposit() public {
        // as Alice
        vm.stopPrank();
        vm.startPrank(alice);

        // Get balance of Alice prior to depositing
        uint256 preBaseBalance = dai.balanceOf(alice);

        // Deposit amount of base
        uint256 depositAmount = 2500e18;
        (uint256 shares, uint256 sharePrice) = hyperdrive.deposit(
            depositAmount,
            true
        );

        // Validate that initial deposits are 1:1
        assertEq(
            shares,
            depositAmount,
            "initial shares should be 1:1 with base"
        );
        assertEq(sharePrice, 1e18, "initial share price should be 1");

        // Validate that tokens have been transferred
        assertEq(
            preBaseBalance - dai.balanceOf(alice),
            depositAmount,
            "hyperdrive should have transferred tokens"
        );
    }

    function test__multiple_deposits() public {
        // as Alice
        vm.stopPrank();
        vm.startPrank(alice);

        // Deposit arbitrary amount in pool
        (uint256 sharesAlice, ) = hyperdrive.deposit(4545e18, true);

        // As, Alice transfer Dai to Bob and fast-forward an arbitrary amount of
        // time accruing arbitrary interest for Alice
        dai.transfer(bob, 1000e18);
        vm.warp(block.timestamp + 1212 days + 54);

        // as Bob
        vm.stopPrank();
        vm.startPrank(bob);

        // Deposit amount of base and fast-forward a year accruing 1% interest
        // for all pooled deposits
        (uint256 sharesBob, ) = hyperdrive.deposit(1000e18, true);
        vm.warp(block.timestamp + 365 days);

        // Get total and per-user amounts of underlying invested
        uint256 underlyingInvested = dsrManager.daiBalance(address(hyperdrive));
        uint256 pricePerShare = hyperdrive.pricePerShare();
        uint256 underlyingForBob = sharesBob.mulDown(pricePerShare);
        uint256 underlyingForAlice = sharesAlice.mulDown(pricePerShare);

        assertApproxEqAbs(
            underlyingForBob,
            1010e18,
            10000,
            "Bob should have accrued approximately 1% interest"
        );
        assertApproxEqAbs(
            underlyingForAlice,
            underlyingInvested - underlyingForBob,
            10000,
            "Alice's shares should reflect all remaining deposits"
        );
    }

    function test__multiple_withdrawals() public {
        // as Alice
        vm.stopPrank();
        vm.startPrank(alice);

        // Deposit arbitrary amount in pool
        (uint256 sharesAlice, ) = hyperdrive.deposit(4545.1115e18, true);

        // As, Alice transfer Dai to Bob and fast-forward an arbitrary amount of
        // time accruing arbitrary interest for Alice
        dai.transfer(bob, 1000e18);
        vm.warp(block.timestamp + 1212 days + 54);

        // as Bob
        vm.stopPrank();
        vm.startPrank(bob);

        // Deposit amount of base and fast-forward a year accruing 1% interest
        // for all pooled deposits
        (uint256 sharesBob, ) = hyperdrive.deposit(1000e18, true);
        vm.warp(block.timestamp + 365 days);

        // Get total and per-user amounts of underlying invested
        uint256 underlyingInvested = dsrManager.daiBalance(address(hyperdrive));

        // Bob should have accrued 1%
        uint256 amountWithdrawnBob = hyperdrive.withdraw(sharesBob, bob, true);
        assertApproxEqAbs(
            amountWithdrawnBob - 1000e18,
            10e18,
            10000,
            "Bob should have accrued approximately 1% interest"
        );

        // Alice shares should make up the rest of the pool
        uint256 amountWithdrawnAlice = hyperdrive.withdraw(
            sharesAlice,
            alice,
            true
        );
        assertApproxEqAbs(
            amountWithdrawnAlice,
            underlyingInvested - amountWithdrawnBob,
            10000,
            "Alice's shares should reflect all remaining deposits"
        );

        assertEq(hyperdrive.totalShares(), 0, "all shares should be exited");
    }

    function test__pricePerShare() public {
        // as Alice
        vm.stopPrank();
        vm.startPrank(alice);

        // Initialize
        hyperdrive.deposit(1000e18, true);

        for (uint256 i = 1; i <= 365; i++) {
            vm.warp(block.timestamp + (0.1 days) * i);

            uint256 pricePerShare = hyperdrive.pricePerShare();

            (, uint256 sharePriceOnDeposit) = hyperdrive.deposit(
                100e18 * i,
                true
            );

            assertApproxEqAbs(
                pricePerShare,
                sharePriceOnDeposit,
                5000,
                "emulated share price should match pool ratio after deposit"
            );
        }
    }

    function test__unsupported_deposit() public {
        vm.expectRevert(IHyperdrive.UnsupportedToken.selector);
        hyperdrive.deposit(1, false);
    }

    function test__unsupported_withdraw() public {
        vm.expectRevert(IHyperdrive.UnsupportedToken.selector);
        hyperdrive.withdraw(1, alice, false);
    }

    // Ensures issue described in https://github.com/delvtech/hyperdrive/issues/357 is patched
    function test_avoids_donation_attack() public {
        vm.stopPrank();
        vm.startPrank(alice);
        uint256 apr = 0.05e18;

        uint256 contribution = 1e5;
        // The pool gets initialized with a minimal contribution
        hyperdrive.initialize(contribution, apr, bob, true);

        // Alice deposits 1e5 DAI to the 0 address
        hyperdrive.addLiquidity(1e5, 0, type(uint256).max, address(0), true);

        // Now totalShares = 2e5
        assertEq(hyperdrive.totalShares(), 2e5 + 1);
        assertEq(dsrManager.daiBalance(address(hyperdrive)), 199998);

        vm.stopPrank();
        vm.startPrank(bob);
        // Bob attempts to rug the pool by removing all liquidity except a small amount of shares
        hyperdrive.removeLiquidity(contribution - 10, 0, bob, true);
        vm.stopPrank();
        vm.startPrank(alice);

        dai.transfer(bob, 2002e18);

        vm.stopPrank();
        vm.startPrank(bob);

        // Bob front-runs Alice with a call to dsrManager.join() with 2000.01 DAI
        dai.approve(address(dsrManager), 2002e18);
        dsrManager.join(address(hyperdrive), 200001e16);

        assertEq(
            dsrManager.daiBalance(address(hyperdrive)),
            2000010000000000100008
        );

        // Some dust leftover
        assertEq(hyperdrive.totalShares(), 1e5 + 11);

        uint256 shareReserves = hyperdrive.getPoolInfo().shareReserves;
        uint256 bondReserves = hyperdrive.getPoolInfo().bondReserves;
        assert(shareReserves != 0);
        assert(bondReserves != 0);
        uint256 initialSharePrice = hyperdrive
            .getPoolConfig()
            .initialSharePrice;
        uint256 positionDuration = hyperdrive.getPoolConfig().positionDuration;
        uint256 timeStretch = hyperdrive.getPoolConfig().timeStretch;

        apr = HyperdriveMath.calculateAPRFromReserves(
            shareReserves,
            bondReserves,
            initialSharePrice,
            positionDuration,
            timeStretch
        );

        vm.stopPrank();
        vm.startPrank(alice);

        // Alice calls addLiquidity() with 1000 DAI
        uint256 newShares = hyperdrive.addLiquidity(
            1000e18,
            apr,
            apr,
            alice,
            true
        );
        // Shares are still minted, and Alice does not get 0 shares out
        assertEq(newShares, 50005);
    }

    // Tests for https://github.com/delvtech/hyperdrive/issues/356
    function testMinimalDeploymentReceivesLiquidity() public {
        vm.stopPrank();
        vm.startPrank(alice);
        uint256 apr = 0.05e18;

        uint256 contribution = 1e5;
        // The pool gets initialized with a minimal contribution
        hyperdrive.initialize(contribution, apr, bob, true);

        uint256 shareReserves = hyperdrive.getPoolInfo().shareReserves;
        uint256 bondReserves = hyperdrive.getPoolInfo().bondReserves;
        assert(shareReserves != 0);
        assert(bondReserves != 0);
        uint256 initialSharePrice = hyperdrive
            .getPoolConfig()
            .initialSharePrice;
        uint256 positionDuration = hyperdrive.getPoolConfig().positionDuration;
        uint256 timeStretch = hyperdrive.getPoolConfig().timeStretch;

        apr = HyperdriveMath.calculateAPRFromReserves(
            shareReserves,
            bondReserves,
            initialSharePrice,
            positionDuration,
            timeStretch
        );

        // Alice calls addLiquidity() with 1000 DAI
        // This would have reverted if minimum contribution was too small to cause
        // division by zero
        hyperdrive.addLiquidity(1000e18, apr, apr, alice, true);
    }

    function testCannotInitializeBelowMinimumContribution() public {
        vm.stopPrank();
        vm.startPrank(alice);
        uint256 apr = 0.05e18;

        uint256 contribution = (1e5) - 1;
        // The pool gets initialized with a minimal contribution
        vm.expectRevert(IHyperdrive.BelowMinimumContribution.selector);
        hyperdrive.initialize(contribution, apr, bob, true);
    }
}

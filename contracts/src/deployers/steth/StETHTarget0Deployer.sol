// SPDX-License-Identifier: Apache-2.0
pragma solidity 0.8.20;

import { StETHTarget0 } from "../../instances/steth/StETHTarget0.sol";
import { IHyperdrive } from "../../interfaces/IHyperdrive.sol";
import { IHyperdriveTargetDeployer } from "../../interfaces/IHyperdriveTargetDeployer.sol";
import { ILido } from "../../interfaces/ILido.sol";

/// @author DELV
/// @title StETHTarget0Deployer
/// @notice The target0 deployer for the StETHHyperdrive implementation.
/// @custom:disclaimer The language used in this code is for coding convenience
///                    only, and is not intended to, and does not, have any
///                    particular legal or regulatory significance.
contract StETHTarget0Deployer is IHyperdriveTargetDeployer {
    /// @notice The Lido contract.
    ILido public immutable lido;

    /// @notice Instantiates the target0 deployer.
    /// @param _lido The Lido contract.
    constructor(ILido _lido) {
        lido = _lido;
    }

    /// @notice Deploys a target0 instance with the given parameters.
    /// @param _config The configuration of the Hyperdrive pool.
    /// @param _salt The create2 salt used in the deployment.
    /// @return The address of the newly deployed StETHTarget0 instance.
    function deploy(
        IHyperdrive.PoolConfig memory _config,
        bytes memory, // unused extra data
        bytes32 _salt
    ) external override returns (address) {
        return
            address(
                // NOTE: We hash the sender with the salt to prevent the
                // front-running of deployments.
                new StETHTarget0{
                    salt: keccak256(abi.encode(msg.sender, _salt))
                }(_config, lido)
            );
    }
}

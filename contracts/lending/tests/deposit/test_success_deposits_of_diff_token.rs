#[cfg(test)]
mod tests {
    use cosmwasm_std::{coin, coins, Addr, Empty};
    use cw_multi_test::{custom_app, ContractWrapper, Executor};
    use std::vec;

    use crate::utils::CustomMsg;
    use collateral_vault::msg::{
        InstantiateMsg as InstantiateMsgCollateralVault, QueryMsg as QueryMsgCollateralVault,
    };
    use collateral_vault::{
        execute as execute_collateral_vault, instantiate as instantiate_collateral_vault,
        query as query_collateral_vault,
    };
    use lending::msg::{ExecuteMsg, GetBalanceResponse, InstantiateMsg, QueryMsg};
    use lending::{execute, instantiate, query};
    use pyth_sdk_cw::PriceIdentifier;
    #[test]
    fn test_successful_deposits_of_diff_token() {
        const TOKENS_DECIMALS: u32 = 18;

        const INIT_BALANCE_FIRST_TOKEN: u128 = 1000 * 10u128.pow(TOKENS_DECIMALS);
        const INIT_BALANCE_SECOND_TOKEN: u128 = 1000 * 10u128.pow(TOKENS_DECIMALS);

        const INIT_BALANCE_FIRST_TOKEN_COLLATERAL_VAULT: u128 = 1000 * 10u128.pow(TOKENS_DECIMALS);

        const INIT_LIQUIDATOR_BALANCE_ETH: u128 = 1_000_000 * 10u128.pow(TOKENS_DECIMALS); // 1M ETH
        const INIT_LIQUIDATOR_BALANCE_ATOM: u128 = 1_000_000 * 10u128.pow(TOKENS_DECIMALS); // 1M ATOM

        const DEPOSIT_OF_FIRST_TOKEN: u128 = 200 * 10u128.pow(TOKENS_DECIMALS);
        const DEPOSIT_OF_SECOND_TOKEN: u128 = 300 * 10u128.pow(TOKENS_DECIMALS);

        const CONTRACT_RESERVES_FIRST_TOKEN: u128 = 1000 * 10u128.pow(TOKENS_DECIMALS);
        const CONTRACT_RESERVES_SECOND_TOKEN: u128 = 1000 * 10u128.pow(TOKENS_DECIMALS);

        const PERCENT_DECIMALS: u32 = 5;
        const LTV_ETH: u128 = 85 * 10u128.pow(PERCENT_DECIMALS); // 85%
        const LIQUIDATION_THRESHOLD_ETH: u128 = 90 * 10u128.pow(PERCENT_DECIMALS); // 90%
        const LTV_ATOM: u128 = 75 * 10u128.pow(PERCENT_DECIMALS); // 75%
        const LIQUIDATION_THRESHOLD_ATOM: u128 = 80 * 10u128.pow(PERCENT_DECIMALS); // 80%

        const INTEREST_RATE_DECIMALS: u32 = 18;
        const MIN_INTEREST_RATE: u128 = 5 * 10u128.pow(INTEREST_RATE_DECIMALS);
        const SAFE_BORROW_MAX_RATE: u128 = 30 * 10u128.pow(INTEREST_RATE_DECIMALS);
        const RATE_GROWTH_FACTOR: u128 = 70 * 10u128.pow(INTEREST_RATE_DECIMALS);

        const OPTIMAL_UTILISATION_RATIO: u128 = 80 * 10u128.pow(PERCENT_DECIMALS);

        let mut lending_contract_app = custom_app::<CustomMsg, Empty, _>(|router, _, storage| {
            router
                .bank
                .init_balance(
                    storage,
                    &Addr::unchecked("user"),
                    vec![
                        coin(INIT_BALANCE_FIRST_TOKEN, "eth"),
                        coin(INIT_BALANCE_SECOND_TOKEN, "atom"),
                    ],
                )
                .unwrap();

            router
                .bank
                .init_balance(
                    storage,
                    &Addr::unchecked("owner"),
                    vec![
                        coin(CONTRACT_RESERVES_FIRST_TOKEN, "eth"),
                        coin(CONTRACT_RESERVES_SECOND_TOKEN, "atom"),
                    ],
                )
                .unwrap();

            router
                .bank
                .init_balance(
                    storage,
                    &Addr::unchecked("liquidator"),
                    vec![
                        coin(INIT_LIQUIDATOR_BALANCE_ETH, "eth"),
                        coin(INIT_LIQUIDATOR_BALANCE_ATOM, "atom"),
                    ],
                )
                .unwrap();

            router
                .bank
                .init_balance(
                    storage,
                    &Addr::unchecked("collateral_vault"),
                    vec![
                        coin(INIT_BALANCE_FIRST_TOKEN_COLLATERAL_VAULT, "eth"),
                        coin(0, "atom"),
                    ],
                )
                .unwrap();
        });

        let code_collateral_vault = ContractWrapper::new_with_empty(
            execute_collateral_vault,
            instantiate_collateral_vault,
            query_collateral_vault,
        );
        let code_id_collateral_vault =
            lending_contract_app.store_code(Box::new(code_collateral_vault));

        let collateral_contract_addr = lending_contract_app
            .instantiate_contract(
                code_id_collateral_vault,
                Addr::unchecked("collateral_vault"),
                &InstantiateMsgCollateralVault {
                    lending_contract: "owner".to_string(),
                    margin_contract: "whatever".to_string(),
                    admin: "collateral_vault".to_string(),
                },
                &[coin(INIT_BALANCE_FIRST_TOKEN_COLLATERAL_VAULT, "eth")],
                "Collateral Vault Contract",
                Some("collateral_vault".to_string()), // contract that can execute migrations
            )
            .unwrap();

        let lending_contract: String = lending_contract_app
            .wrap()
            .query_wasm_smart(
                collateral_contract_addr.clone(),
                &QueryMsgCollateralVault::GetLendingContract {},
            )
            .unwrap();

        assert_eq!(lending_contract, "owner".to_string());

        let margin_contract: String = lending_contract_app
            .wrap()
            .query_wasm_smart(
                collateral_contract_addr.clone(),
                &QueryMsgCollateralVault::GetMarginContract {},
            )
            .unwrap();

        assert_eq!(margin_contract, "whatever".to_string());

        let code = ContractWrapper::new_with_empty(execute, instantiate, query);
        let code_id = lending_contract_app.store_code(Box::new(code));

        let addr = lending_contract_app
            .instantiate_contract(
                code_id,
                Addr::unchecked("owner"),
                &InstantiateMsg {
                    is_testing: true,
                    price_ids: vec![
                        (
                            "inj".to_string(),
                            PriceIdentifier::from_hex(
                                "2d9315a88f3019f8efa88dfe9c0f0843712da0bac814461e27733f6b83eb51b3",
                            )
                            .unwrap(),
                        ),
                        (
                            "peggy0x44C21afAaF20c270EBbF5914Cfc3b5022173FEB7".to_string(),
                            PriceIdentifier::from_hex(
                                "2d9315a88f3019f8efa88dfe9c0f0843712da0bac814461e27733f6b83eb51b3",
                            )
                            .unwrap(),
                        ),
                    ],
                    pyth_contract_addr: "inj1z60tg0tekdzcasenhuuwq3htjcd5slmgf7gpez".to_string(),
                    admin: "owner".to_string(),
                    liquidator: "liquidator".to_string(),
                    supported_tokens: vec![
                        (
                            "eth".to_string(),
                            "ethereum".to_string(),
                            "ETH".to_string(),
                            TOKENS_DECIMALS as u128,
                        ),
                        (
                            "atom".to_string(),
                            "atom".to_string(),
                            "ATOM".to_string(),
                            TOKENS_DECIMALS as u128,
                        ),
                    ],
                    reserve_configuration: vec![
                        ("eth".to_string(), LTV_ETH, LIQUIDATION_THRESHOLD_ETH),
                        ("atom".to_string(), LTV_ATOM, LIQUIDATION_THRESHOLD_ATOM),
                    ],
                    tokens_interest_rate_model_params: vec![
                        (
                            "eth".to_string(),
                            MIN_INTEREST_RATE,
                            SAFE_BORROW_MAX_RATE,
                            RATE_GROWTH_FACTOR,
                            OPTIMAL_UTILISATION_RATIO,
                        ),
                        (
                            "atom".to_string(),
                            MIN_INTEREST_RATE,
                            SAFE_BORROW_MAX_RATE,
                            RATE_GROWTH_FACTOR,
                            OPTIMAL_UTILISATION_RATIO,
                        ),
                    ],
                    price_updater_contract_addr: "".to_string(),
                    collateral_vault_contract: collateral_contract_addr.to_string(),
                },
                &[coin(CONTRACT_RESERVES_SECOND_TOKEN, "atom")],
                "Contract",
                Some("owner".to_string()), // contract that can execute migrations
            )
            .unwrap();

        lending_contract_app
            .execute_contract(
                Addr::unchecked("user"),
                addr.clone(),
                &ExecuteMsg::Deposit {},
                &coins(DEPOSIT_OF_FIRST_TOKEN, "eth"),
            )
            .unwrap();

        let user_deposited_balance: GetBalanceResponse = lending_contract_app
            .wrap()
            .query_wasm_smart(
                addr.clone(),
                &QueryMsg::GetDeposit {
                    address: "user".to_string(),
                    denom: "eth".to_string(),
                },
            )
            .unwrap();

        assert_eq!(
            user_deposited_balance.balance.u128(),
            DEPOSIT_OF_FIRST_TOKEN
        );

        assert_eq!(
            lending_contract_app
                .wrap()
                .query_balance("user", "eth")
                .unwrap()
                .amount
                .u128(),
            INIT_BALANCE_FIRST_TOKEN - DEPOSIT_OF_FIRST_TOKEN
        );

        assert_eq!(
            lending_contract_app
                .wrap()
                .query_balance(collateral_contract_addr.clone(), "eth")
                .unwrap()
                .amount
                .u128(),
            INIT_BALANCE_FIRST_TOKEN_COLLATERAL_VAULT + DEPOSIT_OF_FIRST_TOKEN
        );

        lending_contract_app
            .execute_contract(
                Addr::unchecked("user"),
                addr.clone(),
                &ExecuteMsg::Deposit {},
                &coins(DEPOSIT_OF_SECOND_TOKEN, "atom"),
            )
            .unwrap();

        let user_deposited_balance: GetBalanceResponse = lending_contract_app
            .wrap()
            .query_wasm_smart(
                addr.clone(),
                &QueryMsg::GetDeposit {
                    address: "user".to_string(),
                    denom: "atom".to_string(),
                },
            )
            .unwrap();

        assert_eq!(
            user_deposited_balance.balance.u128(),
            DEPOSIT_OF_SECOND_TOKEN
        );

        assert_eq!(
            lending_contract_app
                .wrap()
                .query_balance("user", "atom")
                .unwrap()
                .amount
                .u128(),
            INIT_BALANCE_SECOND_TOKEN - DEPOSIT_OF_SECOND_TOKEN
        );

        assert_eq!(
            lending_contract_app
                .wrap()
                .query_balance(collateral_contract_addr, "atom")
                .unwrap()
                .amount
                .u128(),
            DEPOSIT_OF_SECOND_TOKEN
        );
    }
}

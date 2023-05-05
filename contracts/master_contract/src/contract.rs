use crate::contract::query::{
    get_available_liquidity_by_token,
    get_available_to_borrow,
    get_available_to_redeem,
    get_borrow_amount_with_interest,
    get_contract_balance_by_token,
    get_current_liquidity_index_ln,
    get_interest_rate,
    get_liquidity_rate,
    get_mm_token_price,
    get_price,
    get_supported_tokens,
    get_token_decimal,
    get_tokens_interest_rate_model_params,
    get_total_borrow_data,
    get_total_borrowed_by_token_usd,
    get_total_deposited_by_token_usd,
    get_total_reserves_by_token,
    get_user_borrowed_usd,
    get_user_borrowing_info,
    get_user_deposited_usd,
    get_utilization_rate_by_token,
//     decimal_to_u128,
};
use crate::msg::{TokenInfo, TokenInterestRateModelParams, UserBorrowingInfo, LiquidityIndexData, TotalBorrowData};
use crate::state::{PRICES, USER_BORROWED_BALANCE, USER_BORROWING_INFO, LIQUIDITY_INDEX_DATA, TOTAL_BORROW_DATA};
use rust_decimal::prelude::{MathematicalOps, Decimal};
// use rust_decimal::Decimal;

use {
    crate::contract::query::get_deposit,
    crate::{
        error::ContractError,
        msg::InstantiateMsg,
        msg::{ExecuteMsg, QueryMsg},
        state::{
            ADMIN, SUPPORTED_TOKENS, TOKENS_INTEREST_RATE_MODEL_PARAMS, USER_DEPOSITED_BALANCE,
        },
    },
    cosmwasm_std::{
        coins, to_binary, BankMsg, Binary, Deps, DepsMut, Env, MessageInfo, Response, StdResult,
        Uint128,
    },
    cw2::set_contract_version,
};
use std::ops::{Mul, Add, Div};
use rust_decimal::prelude::ToPrimitive;

// version info for migration info
const CONTRACT_NAME: &str = "crates.io:master_contract";
const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

const PERCENT_DECIMALS: u32 = 5;
const HUNDRED_PERCENT: u128 = 100 * 10u128.pow(PERCENT_DECIMALS);
const UTILIZATION_LIMIT: u128 = 80 * 10u128.pow(PERCENT_DECIMALS); // 80%

const INTEREST_RATE_DECIMALS: u32 = 18;
const INTEREST_RATE_MULTIPLIER: u128 = 10u128.pow(INTEREST_RATE_DECIMALS);
const HUNDRED: u128 = 100;
const YEAR_IN_SECONDS: u128 = 31536000; // 365 days

pub trait DecimalExt {
    fn rescale_with_return(self, scale: u32) -> Self;
    fn to_u128_with_decimals(&self) -> StdResult<u128>;
}

impl DecimalExt for Decimal {
    fn rescale_with_return(mut self, scale: u32) -> Self {
        self.rescale(scale);
        self
    }

    fn to_u128_with_decimals(&self) -> StdResult<u128> {
        let result = self * Decimal::new(10i64.pow(self.scale()), 0);
        Ok(result.round().to_u128().unwrap())
    }
}

pub fn instantiate(
    deps: DepsMut,
    env: Env,
    _info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;

    ADMIN.save(deps.storage, &msg.admin)?;

    for tokens in msg.supported_tokens {
        SUPPORTED_TOKENS.save(
            deps.storage,
            tokens.0.clone(),
            &TokenInfo {
                denom: tokens.0.clone(),
                name: tokens.1,
                symbol: tokens.2,
                decimals: tokens.3,
            },
        )?;

        TOTAL_BORROW_DATA.save(
            deps.storage,
            tokens.0.clone(),
            &TotalBorrowData {
                denom: tokens.0.clone(),
                total_borrowed_amount: 0u128,
                total_borrowed_interest: 0u128,
            },
        )?;

        LIQUIDITY_INDEX_DATA.save(
            deps.storage,
            tokens.0.clone(),
            &LiquidityIndexData {
                denom: tokens.0.clone(),
                liquidity_index_ln: 0u128,
                timestamp: env.block.time,
            },
        )?;
    }

    for params in msg.tokens_interest_rate_model_params {
        TOKENS_INTEREST_RATE_MODEL_PARAMS.save(
            deps.storage,
            params.0.clone(),
            &TokenInterestRateModelParams {
                denom: params.0,
                min_interest_rate: params.1,
                safe_borrow_max_rate: params.2,
                rate_growth_factor: params.3,
            },
        )?;
    }

    Ok(Response::new().add_attribute("method", "instantiate"))
}

pub fn execute(
    mut deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, ContractError> {
    match msg {
        ExecuteMsg::Deposit {} => {
            // underlying balances that are allowing user to send funds along with calling Deposit
            // is operated through cw20 contracts

            if info.funds.is_empty() {
                return Err(ContractError::CustomError {
                    val: "No funds deposited!".to_string(),
                });
            }

            assert_eq!(
                info.funds.len(),
                1,
                "You have to deposit one asset per time"
            );

            let allowed_coin = info.funds.first().unwrap();

            assert!(allowed_coin.amount.u128() > 0);

            assert!(
                SUPPORTED_TOKENS.has(deps.storage, allowed_coin.denom.clone()),
                "There is no such supported token yet"
            );

            execute_update_liquidity_index_data(
                &mut deps,
                env.clone(),
                allowed_coin.denom.clone(),
            )?;

            let current_balance = get_deposit(
                deps.as_ref(),
                env.clone(),
                info.sender.to_string(),
                allowed_coin.denom.clone(),
            )
            .unwrap();
            let new_balance = current_balance.balance.u128() + allowed_coin.amount.u128();
            USER_DEPOSITED_BALANCE.save(
                deps.storage,
                (info.sender.to_string(), allowed_coin.denom.clone()),
                &Uint128::from(new_balance),
            )?;

            Ok(Response::default())
        }
        ExecuteMsg::Redeem { denom, amount } => {
            assert!(amount.u128() > 0, "Amount should be a positive number");

            assert!(
                SUPPORTED_TOKENS.has(deps.storage, denom.clone()),
                "There is no such supported token yet"
            );

            execute_update_liquidity_index_data(
                &mut deps,
                env.clone(),
                denom.clone(),
            )?;

            let current_balance =
                query::get_deposit(deps.as_ref(), env.clone(), info.sender.to_string(), denom.clone())?;

            let amount = amount.u128();

            assert!(
                current_balance.balance.u128() >= amount,
                "The account doesn't have enough digital tokens to do withdraw"
            );

            let remaining = current_balance.balance.u128() - amount;

            USER_DEPOSITED_BALANCE.save(
                deps.storage,
                (info.sender.to_string(), denom.clone()),
                &Uint128::from(remaining),
            )?;

            Ok(Response::new().add_message(BankMsg::Send {
                to_address: info.sender.to_string(),
                amount: coins(amount, denom.clone()),
            }))
        }
        ExecuteMsg::Fund {} => {
            if info.funds.is_empty() {
                return Err(ContractError::CustomError {
                    val: "No funds deposited!".to_string(),
                });
            }

            assert_eq!(
                info.sender.to_string(),
                ADMIN.load(deps.storage).unwrap(),
                "This functionality is allowed for admin only"
            );

            Ok(Response::default())
        }
        ExecuteMsg::AddMarkets {
            denom,
            name,
            symbol,
            decimals,
            min_interest_rate,
            safe_borrow_max_rate,
            rate_growth_factor,
        } => {
            SUPPORTED_TOKENS.save(
                deps.storage,
                denom.clone(),
                &TokenInfo {
                    denom: denom.clone(),
                    name,
                    symbol,
                    decimals,
                },
            )?;

            TOKENS_INTEREST_RATE_MODEL_PARAMS.save(
                deps.storage,
                denom.clone(),
                &TokenInterestRateModelParams {
                    denom: denom.clone(),
                    min_interest_rate,
                    safe_borrow_max_rate,
                    rate_growth_factor,
                },
            )?;

            TOTAL_BORROW_DATA.save(
                deps.storage,
                denom.clone(),
                &TotalBorrowData {
                    denom: denom.clone(),
                    total_borrowed_amount: 0u128,
                    total_borrowed_interest: 0u128,
                },
            )?;

            LIQUIDITY_INDEX_DATA.save(
                deps.storage,
                denom.clone(),
                &LiquidityIndexData {
                    denom: denom.clone(),
                    liquidity_index_ln: 0u128,
                    timestamp: env.block.time,
                },
            )?;

            Ok(Response::default())
        }
        ExecuteMsg::Borrow { denom, amount } => {
            assert!(
                SUPPORTED_TOKENS.has(deps.storage, denom.clone()),
                "There is no such supported token yet"
            );

            let available_to_borrow_amount = get_available_to_borrow(
                deps.as_ref(),
                env.clone(),
                info.sender.to_string(),
                denom.clone(),
            )
            .unwrap()
            .u128();

            assert!(
                available_to_borrow_amount >= amount.u128(),
                "User don't have enough deposit to borrow that much"
            );

            assert!(
                get_contract_balance_by_token(deps.as_ref(), env.clone(), denom.clone())
                    .unwrap()
                    .u128()
                    >= amount.u128()
            );

            execute_update_liquidity_index_data(
                &mut deps,
                env.clone(),
                denom.clone(),
            )?;

            let borrow_amount_with_interest = get_borrow_amount_with_interest(
                deps.as_ref(),
                env.clone(),
                info.sender.to_string(),
                denom.clone(),
            )
                .unwrap()
                .amount
                .u128();

            let user_borrowing_info = get_user_borrowing_info(
                deps.as_ref(),
                info.sender.to_string().clone(),
                denom.clone(),
            )
                .unwrap_or_default();

            let new_user_borrow_amount: u128 = borrow_amount_with_interest + amount.u128();

            let current_interest_rate =
                get_interest_rate(deps.as_ref(), env.clone(), denom.clone())
                    .unwrap()
                    .u128();

            let borrowed_token_decimals =
                get_token_decimal(deps.as_ref(), denom.clone())
                    .unwrap()
                    .u128() as u32;

            let average_interest_rate = (
                  Decimal::from_i128_with_scale(
                      borrow_amount_with_interest as i128,
                      borrowed_token_decimals,
                  )
                      .rescale_with_return(INTEREST_RATE_DECIMALS)
                      .mul(
                          Decimal::from_i128_with_scale(
                              user_borrowing_info.average_interest_rate as i128,
                              INTEREST_RATE_DECIMALS,
                          )
                      )
                      .add(
                          Decimal::from_i128_with_scale(
                              amount.u128() as i128,
                              borrowed_token_decimals,
                          )
                              .rescale_with_return(INTEREST_RATE_DECIMALS)
                              .mul(
                                  Decimal::from_i128_with_scale(
                                      current_interest_rate as i128,
                                      INTEREST_RATE_DECIMALS,
                                  )
                              )
                      )
              ).div(
                  Decimal::from_i128_with_scale(
                      new_user_borrow_amount as i128,
                      borrowed_token_decimals,
                  )
                      .rescale_with_return(INTEREST_RATE_DECIMALS)
              );

            // updating user borrowing info
            let new_user_borrowing_info = UserBorrowingInfo {
                borrowed_amount: Uint128::from(new_user_borrow_amount.clone()),
                accumulated_interest: Uint128::from(new_user_borrow_amount.clone() / 8),
                average_interest_rate: average_interest_rate.to_u128_with_decimals().unwrap(),
                timestamp: env.block.time,
            };

            let total_borrow_data = get_total_borrow_data(
                deps.as_ref(),
                denom.clone(),
            )
            .unwrap_or_default();

            let total_borrowed_interest = total_borrow_data.total_borrowed_interest
                - Decimal::from_i128_with_scale(
                      user_borrowing_info.borrowed_amount.u128() as i128,
                      borrowed_token_decimals,
                  )
                      .rescale_with_return(INTEREST_RATE_DECIMALS)
                      .mul(
                          Decimal::from_i128_with_scale(
                              user_borrowing_info.average_interest_rate as i128,
                              INTEREST_RATE_DECIMALS,
                          )
                      )
                      .rescale_with_return(borrowed_token_decimals)
                      .to_u128_with_decimals()
                      .unwrap()
                + Decimal::from_i128_with_scale(
                      new_user_borrow_amount as i128,
                      borrowed_token_decimals,
                  )
                      .rescale_with_return(INTEREST_RATE_DECIMALS)
                      .mul(average_interest_rate)
                      .rescale_with_return(borrowed_token_decimals as u32)
                      .to_u128_with_decimals()
                      .unwrap();

            // updating total borrow data
            let new_total_borrow_data = TotalBorrowData {
                denom: denom.clone(),
                total_borrowed_amount: total_borrow_data.total_borrowed_amount
                    - user_borrowing_info.borrowed_amount.u128()
                    + new_user_borrow_amount.clone(),
                total_borrowed_interest: total_borrowed_interest,
            };

            USER_BORROWING_INFO.save(
                deps.storage,
                (info.sender.to_string(), denom.clone()),
                &new_user_borrowing_info,
            )?;

            TOTAL_BORROW_DATA.save(
                deps.storage,
                denom.clone(),
                &new_total_borrow_data,
            )?;

            Ok(Response::new().add_message(BankMsg::Send {
                to_address: info.sender.to_string(),
                amount: coins(amount.u128(), denom.clone()),
            }))
        }
        ExecuteMsg::SetPrice { denom, price } => {
            assert_eq!(
                info.sender.to_string(),
                ADMIN.load(deps.storage).unwrap(),
                "This functionality is allowed for admin only"
            );

            PRICES.save(deps.storage, denom, &price)?;

            Ok(Response::new())
        }
        ExecuteMsg::Repay {} => {
            if info.funds.is_empty() {
                return Err(ContractError::CustomError {
                    val: "No funds deposited!".to_string(),
                });
            }

            assert_eq!(info.funds.len(), 1, "You have to repay one asset per time");

            let repay_token = info.funds.first().unwrap();

            assert!(repay_token.amount.u128() > 0);

            assert!(
                SUPPORTED_TOKENS.has(deps.storage, repay_token.denom.clone()),
                "There is no such token to repay yet"
            );
            let current_borrowing_info = get_user_borrowing_info(
                deps.as_ref(),
                info.sender.to_string().clone(),
                repay_token.denom.clone(),
            )
            .unwrap_or_default();

            execute_update_liquidity_index_data(
                &mut deps,
                env.clone(),
                repay_token.denom.clone(),
            )?;

            let borrow_amount_with_interest = get_borrow_amount_with_interest(
                deps.as_ref(),
                env.clone(),
                info.sender.to_string(),
                repay_token.denom.clone(),
            )
            .unwrap()
            .amount;

            if repay_token.amount.u128() < current_borrowing_info.accumulated_interest.u128() {
                let new_user_borrowing_info = UserBorrowingInfo {
                    borrowed_amount: current_borrowing_info.borrowed_amount,
                    accumulated_interest: Uint128::from(
                        current_borrowing_info.accumulated_interest.u128()
                            - repay_token.amount.u128(),
                    ),
                    average_interest_rate: current_borrowing_info.average_interest_rate,
                    timestamp: env.block.time,
                };

                USER_BORROWING_INFO.save(
                    deps.storage,
                    (info.sender.to_string(), repay_token.denom.clone()),
                    &new_user_borrowing_info,
                )?;
                Ok(Response::default())
            } else if repay_token.amount.u128()
                <= current_borrowing_info.accumulated_interest.u128()
                    + borrow_amount_with_interest.u128()
            {
                let new_borrow_amount = borrow_amount_with_interest.u128()
                    - (repay_token.amount.u128()
                        - current_borrowing_info.accumulated_interest.u128());
                let new_user_borrowing_info = UserBorrowingInfo {
                    borrowed_amount: Uint128::from(new_borrow_amount),
                    accumulated_interest: Default::default(),
                    average_interest_rate: current_borrowing_info.average_interest_rate,
                    timestamp: env.block.time,
                };

                USER_BORROWING_INFO.save(
                    deps.storage,
                    (info.sender.to_string(), repay_token.denom.clone()),
                    &new_user_borrowing_info,
                )?;

                USER_BORROWED_BALANCE.save(
                    deps.storage,
                    (info.sender.to_string(), repay_token.denom.clone()),
                    &Uint128::from(new_borrow_amount),
                )?;
                Ok(Response::default())
            } else {
                USER_BORROWING_INFO.save(
                    deps.storage,
                    (info.sender.to_string(), repay_token.denom.clone()),
                    &UserBorrowingInfo::default(),
                )?;

                USER_BORROWED_BALANCE.save(
                    deps.storage,
                    (info.sender.to_string(), repay_token.denom.clone()),
                    &Uint128::zero(),
                )?;
                // send back if all borrowed was repaid
                let remaining = repay_token.amount.u128()
                    - current_borrowing_info.accumulated_interest.u128()
                    - borrow_amount_with_interest.u128();

                Ok(Response::new().add_message(BankMsg::Send {
                    to_address: info.sender.to_string(),
                    amount: coins(remaining, repay_token.denom.clone()),
                }))
            }
        }
    }
}

pub fn execute_update_liquidity_index_data(
    deps: &mut DepsMut,
    env: Env,
    denom: String,
) -> StdResult<Response> {
    let current_liquidity_index_ln = get_current_liquidity_index_ln(deps.as_ref(), env.clone(), denom.clone())
        .unwrap()
        .u128();

    let new_liquidity_index_data = LiquidityIndexData {
        denom: denom.clone(),
        liquidity_index_ln: current_liquidity_index_ln,
        timestamp: env.block.time,
    };

    LIQUIDITY_INDEX_DATA.save(
        deps.storage,
        denom.clone(),
        &new_liquidity_index_data,
    )?;

    Ok(Response::default())
}

pub fn query(deps: Deps, env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::GetDeposit { address, denom } => to_binary(&get_deposit(deps, env, address, denom)?),
        QueryMsg::GetPrice { denom } => to_binary(&get_price(deps, denom)?),
        QueryMsg::GetBorrowAmountWithInterest { address, denom } => to_binary(
            &query::get_borrow_amount_with_interest(deps, env, address, denom)?,
        ),
        QueryMsg::GetUserBorrowingInfo { address, denom } => {
            to_binary(&query::get_user_borrowing_info(deps, address, denom)?)
        }
        QueryMsg::GetTotalBorrowData { denom } => {
            to_binary(&query::get_total_borrow_data(deps, denom)?)
        }
        QueryMsg::GetSupportedTokens {} => to_binary(&get_supported_tokens(deps)?),
        QueryMsg::GetTokensInterestRateModelParams {} => {
            to_binary(&get_tokens_interest_rate_model_params(deps)?)
        }
        QueryMsg::GetInterestRate { denom } => to_binary(&get_interest_rate(deps, env, denom)?),
        QueryMsg::GetLiquidityRate { denom } => to_binary(&get_liquidity_rate(deps, env, denom)?),
        QueryMsg::GetCurrentLiquidityIndexLn { denom } => to_binary(&get_current_liquidity_index_ln(deps, env, denom)?),
        QueryMsg::GetMmTokenPrice { denom } => to_binary(&get_mm_token_price(deps, env, denom)?),
        QueryMsg::GetUserDepositedUsd { address } => {
            to_binary(&get_user_deposited_usd(deps, env, address)?)
        }
        QueryMsg::GetUserBorrowedUsd { address } => {
            to_binary(&get_user_borrowed_usd(deps, env, address)?)
        }
        QueryMsg::GetContractBalance { denom } => {
            to_binary(&get_contract_balance_by_token(deps, env, denom)?)
        }
        QueryMsg::GetAvailableToBorrow { address, denom } => {
            to_binary(&get_available_to_borrow(deps, env, address, denom)?)
        }
        QueryMsg::GetAvailableToRedeem { address, denom } => {
            to_binary(&get_available_to_redeem(deps, env, address, denom)?)
        }
        QueryMsg::GetTotalReservesByToken { denom } => {
            to_binary(&get_total_reserves_by_token(deps, env, denom)?)
        }
        QueryMsg::GetTotalDepositedByToken { denom } => {
            to_binary(&get_total_deposited_by_token_usd(deps, denom)?)
        }
        QueryMsg::GetTotalBorrowedByToken { denom } => {
            to_binary(&get_total_borrowed_by_token_usd(deps, denom)?)
        }
        QueryMsg::GetAvailableLiquidityByToken { denom } => {
            to_binary(&get_available_liquidity_by_token(deps, env, denom)?)
        }
        QueryMsg::GetUtilizationRateByToken { denom } => {
            to_binary(&get_utilization_rate_by_token(deps, env, denom)?)
        }
    }
}
    
pub mod query {
    use super::*;
//     use std::fmt::Debug;
    use std::ops::{Mul};

//     use std::str::FromStr;

    use crate::msg::{
        GetBalanceResponse,
        GetBorrowAmountWithInterestResponse,
        GetPriceResponse,
        GetSupportedTokensResponse,
        GetTokensInterestRateModelParamsResponse,
        GetUserBorrowedUsdResponse,
        GetUserDepositedUsdResponse,
        UserBorrowingInfo,
        TotalBorrowData
    };
    use cosmwasm_std::{Coin, Order};
//     use near_sdk::json_types::U128;
//     use rust_decimal::prelude::ToPrimitive;

    pub fn get_deposit(
        deps: Deps,
        env: Env,
        user: String,
        denom: String
    ) -> StdResult<GetBalanceResponse> {
        let token_decimals =
            get_token_decimal(deps, denom.clone())
                .unwrap()
                .u128() as u32;

        let user_mm_token_balance = USER_DEPOSITED_BALANCE
            .load(deps.storage, (user, denom.clone()))
            .unwrap_or_else(|_| Uint128::zero());

        let mm_token_price = 
            get_mm_token_price(deps.clone(), env.clone(), denom.clone())
                .unwrap()
                .u128();

        let user_token_balance = 
            Decimal::from_i128_with_scale(
                user_mm_token_balance.u128() as i128,
                token_decimals,
            )
                .mul(
                    Decimal::from_i128_with_scale(
                        mm_token_price as i128,
                        token_decimals,
                    )
                )
                .to_u128_with_decimals()
                .unwrap();

        Ok(GetBalanceResponse { balance: Uint128::from(user_token_balance) })
    }

    pub fn get_borrow_amount_with_interest(
        deps: Deps,
        env: Env,
        user: String,
        denom: String,
    ) -> StdResult<GetBorrowAmountWithInterestResponse> {
        let current_borrowing_info =
            get_user_borrowing_info(deps.clone(), user.clone(), denom.clone()).unwrap_or_default();

        let token_decimals =
            get_token_decimal(deps, denom.clone())
                .unwrap()
                .u128() as u32;

        let current_borrowed_amount = Decimal::from_i128_with_scale(
            current_borrowing_info.borrowed_amount.u128() as i128,
            token_decimals,
        ).rescale_with_return(INTEREST_RATE_DECIMALS);

        let base = Decimal::from_i128_with_scale(
            (current_borrowing_info.average_interest_rate / HUNDRED + INTEREST_RATE_MULTIPLIER)
                as i128,
            INTEREST_RATE_DECIMALS,
        );

        let exponent = Decimal::from_i128_with_scale(
            ((env.block.time.seconds() - current_borrowing_info.timestamp.seconds()) as u128
                * INTEREST_RATE_MULTIPLIER
                / YEAR_IN_SECONDS) as i128,
            INTEREST_RATE_DECIMALS,
        );

        let borrow_amount_with_interest =
            current_borrowed_amount
                .mul(base.powd(exponent))
                .rescale_with_return(token_decimals)
                .to_u128_with_decimals()
                .unwrap();

        Ok(GetBorrowAmountWithInterestResponse {
            amount: Uint128::from(borrow_amount_with_interest),
        })
    }

//     pub fn decimal_to_u128(d: Decimal) -> StdResult<u128> {
//         let result = d * Decimal::new(10i64.pow(d.scale()), 0);
//         Ok(result.round().to_u128().unwrap())
//     }

    pub fn get_liquidity_rate(
        deps: Deps,
        env: Env,
        denom: String,
    ) -> StdResult<Uint128> {
        let total_borrowed_interest = TOTAL_BORROW_DATA
            .load(deps.storage, denom.clone())
            .unwrap()
            .total_borrowed_interest;

        let reserves_by_token = get_total_reserves_by_token(deps, env.clone(), denom.clone())
            .unwrap()
            .u128();

        if reserves_by_token == 0 {
            Ok(Uint128::from(0u128))
        } else {
            Ok(Uint128::from(total_borrowed_interest/reserves_by_token))
        }
    }

    pub fn get_current_liquidity_index_ln(
        deps: Deps,
        env: Env,
        denom: String,
    ) -> StdResult<Uint128> {
        let liquidity_rate = get_liquidity_rate(deps, env.clone(), denom.clone())
            .unwrap()
            .u128();

        let liquidity_index_last_update = LIQUIDITY_INDEX_DATA
            .load(deps.storage, denom.clone())
            .unwrap()
            .timestamp;

        let liquidity_index_ln = LIQUIDITY_INDEX_DATA
            .load(deps.storage, denom.clone())
            .unwrap()
            .liquidity_index_ln;

        let new_liquidity_index_ln = (env.block.time.seconds() - liquidity_index_last_update.seconds()) as u128
            * Decimal::from_i128_with_scale(
                  (liquidity_rate / HUNDRED + INTEREST_RATE_MULTIPLIER) as i128,
                  INTEREST_RATE_DECIMALS,
              )
                  .ln()
                  .to_u128_with_decimals()
                  .unwrap()
            / YEAR_IN_SECONDS
            + liquidity_index_ln;

        Ok(Uint128::from(new_liquidity_index_ln))
    }

//     function cumulatedAmount(address _asset, uint256 _storedAmount) public view returns (uint256) {
//         return _storedAmount * getLiquidityIndexLog2(_asset).exp2() / DECIMALS_MULTIPLIER;
//     }
// 
//     function storedAmount(address _asset, uint256 _cumulatedAmount) external view returns (uint256) {
//         return _cumulatedAmount * DECIMALS_MULTIPLIER / getLiquidityIndexLog2(_asset).exp2();
//     }

    // number of tokens that correspond to one mmToken
    pub fn get_mm_token_price(
        deps: Deps,
        env: Env,
        denom: String,
    ) -> StdResult<Uint128> {
        let token_decimals =
            get_token_decimal(deps, denom.clone())
                .unwrap()
                .u128() as u32;

        let current_liquidity_index_ln = get_current_liquidity_index_ln(deps, env.clone(), denom.clone())
            .unwrap()
            .u128();

        let mm_token_price =
            Decimal::from_i128_with_scale(
                10u128.pow(token_decimals) as i128,
                token_decimals,
            )
                .rescale_with_return(INTEREST_RATE_DECIMALS)
                .mul(
                    Decimal::from_i128_with_scale(
                        current_liquidity_index_ln as i128,
                        INTEREST_RATE_DECIMALS,
                    )
                        .exp()
                )
                .rescale_with_return(token_decimals)
                .to_u128_with_decimals()
                .unwrap_or_default();

        Ok(Uint128::from(mm_token_price))
    }

    pub fn get_price(deps: Deps, denom: String) -> StdResult<GetPriceResponse> {
        let price = PRICES.load(deps.storage, denom).unwrap_or(0u128);

        Ok(GetPriceResponse { price })
    }

    pub fn get_supported_tokens(deps: Deps) -> StdResult<GetSupportedTokensResponse> {
        let mut result: Vec<TokenInfo> = vec![];

        let all: StdResult<Vec<_>> = SUPPORTED_TOKENS
            .range(deps.storage, None, None, Order::Ascending)
            .collect();
        for el in all.unwrap() {
            result.push(el.1)
        }

        Ok(GetSupportedTokensResponse {
            supported_tokens: result,
        })
    }

    pub fn get_tokens_interest_rate_model_params(
        deps: Deps,
    ) -> StdResult<GetTokensInterestRateModelParamsResponse> {
        let mut result: Vec<TokenInterestRateModelParams> = vec![];

        let all: StdResult<Vec<_>> = TOKENS_INTEREST_RATE_MODEL_PARAMS
            .range(deps.storage, None, None, Order::Ascending)
            .collect();
        for el in all.unwrap() {
            result.push(el.1)
        }

        Ok(GetTokensInterestRateModelParamsResponse {
            tokens_interest_rate_model_params: result,
        })
    }

    pub fn get_interest_rate(
        deps: Deps,
        env: Env,
        denom: String,
    ) -> StdResult<Uint128> {
        let utilization_rate = get_utilization_rate_by_token(deps, env.clone(), denom.clone())
            .unwrap()
            .u128();

        let min_interest_rate = TOKENS_INTEREST_RATE_MODEL_PARAMS
            .load(deps.storage, denom.clone())
            .unwrap()
            .min_interest_rate;
        let safe_borrow_max_rate = TOKENS_INTEREST_RATE_MODEL_PARAMS
            .load(deps.storage, denom.clone())
            .unwrap()
            .safe_borrow_max_rate;
        let rate_growth_factor = TOKENS_INTEREST_RATE_MODEL_PARAMS
            .load(deps.storage, denom.clone())
            .unwrap()
            .rate_growth_factor;

        if utilization_rate <= UTILIZATION_LIMIT {
            Ok(Uint128::from(
                min_interest_rate
                    + utilization_rate
                        * (safe_borrow_max_rate - min_interest_rate)
                        / UTILIZATION_LIMIT
            ))
        } else {
            Ok(Uint128::from(
                safe_borrow_max_rate
                    + rate_growth_factor
                        * (utilization_rate - UTILIZATION_LIMIT)
                        / (HUNDRED_PERCENT - UTILIZATION_LIMIT)
            ))
        }
    }

    pub fn get_token_decimal(deps: Deps, denom: String) -> StdResult<Uint128> {
        // contract only inner call, so there is no need to parse non-existent token denom
        Ok(Uint128::from(
            SUPPORTED_TOKENS.load(deps.storage, denom).unwrap().decimals,
        ))
    }

    pub fn get_user_borrowing_info(
        deps: Deps,
        user: String,
        denom: String,
    ) -> StdResult<UserBorrowingInfo> {
        Ok(USER_BORROWING_INFO
            .load(deps.storage, (user, denom))
            .unwrap_or_default())
    }

    pub fn get_total_borrow_data(
        deps: Deps,
        denom: String,
    ) -> StdResult<TotalBorrowData> {
        Ok(TOTAL_BORROW_DATA
            .load(deps.storage, denom)
            .unwrap_or_default())
    }

    pub fn get_user_deposited_usd(
        deps: Deps,
        env: Env,
        user: String,
    ) -> StdResult<GetUserDepositedUsdResponse> {
        let mut user_deposited_usd = 0u128;
        for tokens in get_supported_tokens(deps).unwrap().supported_tokens {
            let user_deposit = get_deposit(deps, env.clone(), user.clone(), tokens.denom.clone())
                .unwrap()
                .balance
                .u128();

            match get_token_decimal(deps, tokens.denom.clone())
                .unwrap()
                .u128()
            {
                18 => {
                    user_deposited_usd +=
                        user_deposit * get_price(deps, tokens.denom).unwrap().price;
                }
                6 => {
                    let user_deposit = user_deposit / 10u128.pow(6) * 10u128.pow(18);
                    user_deposited_usd +=
                        user_deposit * get_price(deps, tokens.denom.clone()).unwrap().price;
                }
                _ => {}
            }
        }

        Ok(GetUserDepositedUsdResponse {
            user_deposited_usd: Uint128::from(user_deposited_usd),
        })
    }

    pub fn get_user_borrowed_usd(
        deps: Deps,
        env: Env,
        user: String,
    ) -> StdResult<GetUserBorrowedUsdResponse> {
        let mut user_borrowed_usd = 0u128;
        for tokens in get_supported_tokens(deps).unwrap().supported_tokens {
            let user_borrow = get_borrow_amount_with_interest(
                deps,
                env.clone(),
                user.clone(),
                tokens.denom.clone(),
            )
            .unwrap()
            .amount
            .u128();

            match get_token_decimal(deps, tokens.denom.clone())
                .unwrap()
                .u128()
            {
                18 => {
                    user_borrowed_usd += user_borrow * get_price(deps, tokens.denom).unwrap().price;
                }
                6 => {
                    let user_borrow = user_borrow / 10u128.pow(6) * 10u128.pow(18);
                    user_borrowed_usd +=
                        user_borrow * get_price(deps, tokens.denom.clone()).unwrap().price;
                }
                _ => {}
            }
        }

        Ok(GetUserBorrowedUsdResponse {
            user_borrowed_usd: Uint128::from(user_borrowed_usd),
        })
    }

    pub fn get_contract_balance_by_token(
        deps: Deps,
        env: Env,
        denom: String,
    ) -> StdResult<Uint128> {
        let contract_address = env.contract.address;
        let coins: Vec<Coin> = deps.querier.query_all_balances(contract_address)?;

        let balance = coins
            .into_iter()
            .find(|coin| coin.denom == denom)
            .map_or(Uint128::zero(), |coin| coin.amount);

        Ok(balance)
    }

    pub fn get_available_to_borrow(
        deps: Deps,
        env: Env,
        user: String,
        denom: String,
    ) -> StdResult<Uint128> {
        let deposited_amount_in_usd = get_user_deposited_usd(deps, env.clone(), user.clone())
            .unwrap()
            .user_deposited_usd
            .u128();

        // amount available to borrow
        let available_to_borrow_usd = deposited_amount_in_usd * 8u128 / 10u128;

        let available_to_borrow_amount =
            available_to_borrow_usd / get_price(deps, denom.clone()).unwrap().price;

        Ok(Uint128::from(
            available_to_borrow_amount
                - get_borrow_amount_with_interest(deps, env, user, denom)
                    .unwrap()
                    .amount
                    .u128(),
        ))
    }

    pub fn get_available_to_redeem(
        deps: Deps,
        env: Env,
        user: String,
        denom: String,
    ) -> StdResult<Uint128> {
        let mut available_to_redeem = 0u128;

        let sum_collateral_balance_usd = get_user_deposited_usd(deps, env.clone(), user.clone())
            .unwrap()
            .user_deposited_usd
            .u128();

        let user_deposit_in_that_token = get_deposit(deps, env.clone(), user.clone(), denom.clone())
            .unwrap()
            .balance
            .u128();

        if user_deposit_in_that_token == 0 {
            available_to_redeem = 0;
        } else {
            let sum_borrow_balance_usd = get_user_borrowed_usd(deps, env.clone(), user.clone())
                .unwrap()
                .user_borrowed_usd
                .u128();

            if sum_borrow_balance_usd <= sum_collateral_balance_usd * 8u128 / 10u128 {
                let borrow_amount_with_interest =
                    get_borrow_amount_with_interest(deps, env.clone(), user.clone(), denom.clone())
                        .unwrap()
                        .amount
                        .u128();

                available_to_redeem = (sum_collateral_balance_usd
                    - sum_borrow_balance_usd * 10u128 / 8u128)
                    / get_price(deps, denom.clone()).unwrap().price
                    - borrow_amount_with_interest;
            } else if sum_borrow_balance_usd == 0 {
                available_to_redeem = get_deposit(deps, env.clone(), user.clone(), denom.clone())
                    .unwrap()
                    .balance
                    .u128();
            }
        }

        Ok(Uint128::from(available_to_redeem))
    }

    pub fn get_total_deposited_by_token_usd(deps: Deps, denom: String) -> StdResult<Uint128> {
        let mut all_deposits_usd = 0u128;
        let all_deposits_iter: StdResult<Vec<_>> = USER_DEPOSITED_BALANCE
            .range(deps.storage, None, None, Order::Ascending)
            .collect();

        for deposits in all_deposits_iter.unwrap() {
            if deposits.0 .1 == denom {
                all_deposits_usd +=
                    deposits.1.u128() * get_price(deps, deposits.0 .1).unwrap().price;
            }
        }

        Ok(Uint128::from(all_deposits_usd))
    }

    pub fn get_total_borrowed_by_token_usd(deps: Deps, denom: String) -> StdResult<Uint128> {
        let mut all_borrowed_usd = 0u128;
        let all_borrowed_iter: StdResult<Vec<_>> = USER_BORROWED_BALANCE
            .range(deps.storage, None, None, Order::Ascending)
            .collect();

        for borrows in all_borrowed_iter.unwrap() {
            if borrows.0 .1 == denom {
                all_borrowed_usd += borrows.1.u128() * get_price(deps, borrows.0 .1).unwrap().price;
            }
        }

        Ok(Uint128::from(all_borrowed_usd))
    }

    pub fn get_total_reserves_by_token(deps: Deps, env: Env, denom: String) -> StdResult<Uint128> {
        let contract_balance = get_contract_balance_by_token(deps, env, denom.clone())
            .unwrap()
            .u128();
        let borrowed_by_token = get_total_borrowed_by_token_usd(deps, denom.clone())
            .unwrap()
            .u128();

        Ok(Uint128::from(contract_balance + borrowed_by_token))
    }

    pub fn get_available_liquidity_by_token(
        deps: Deps,
        env: Env,
        denom: String,
    ) -> StdResult<Uint128> {
        let contract_balance = get_contract_balance_by_token(deps, env.clone(), denom.clone())
            .unwrap()
            .u128();

        Ok(Uint128::from(contract_balance))
    }

    pub fn get_utilization_rate_by_token(
        deps: Deps,
        env: Env,
        denom: String,
    ) -> StdResult<Uint128> {
        let reserves_by_token = get_total_reserves_by_token(deps, env.clone(), denom.clone())
            .unwrap()
            .u128();

        let borrowed_by_token = get_total_borrowed_by_token_usd(deps, denom.clone())
            .unwrap()
            .u128();

        if reserves_by_token != 0 {
            Ok(Uint128::from(
                borrowed_by_token * HUNDRED_PERCENT / reserves_by_token,
            ))
        } else {
            Ok(Uint128::from(0u128))
        }
    }
}

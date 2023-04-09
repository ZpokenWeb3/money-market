use cosmwasm_schema::{cw_serde, QueryResponses};
use cosmwasm_std::{Addr, Uint128};


#[cw_serde]
pub struct InstantiateMsg {
    pub vault: String,
    pub denom: String,
}

#[cw_serde]
pub enum ExecuteMsg {
    Deposit {},
    Withdraw { denom: String, amount: Uint128 },
}

#[cw_serde]
pub enum QueryMsg {
    GetDeposit { address: String, denom: String }
}
#[cfg(not(feature = "library"))]
use cosmwasm_std::entry_point;
use cosmwasm_std::{to_binary, Binary, Deps, DepsMut, Env, MessageInfo, Response, StdResult, CosmosMsg, WasmMsg, Coin, Order};

use crate::error::ContractError;
use crate::msg::{ExecuteMsg, InstantiateMsg, QueryMsg, PaymentsResponse, Payment};
use crate::state::{PaymentState, PAYMENTS, next_id};
use cw20::Cw20ExecuteMsg;
use cw_storage_plus::Map;

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn instantiate(
    deps: DepsMut,
    _env: Env,
    info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    println!("{}",msg.schedule.len());
    for p in msg.schedule.clone().into_iter() {
       let id = next_id(deps.storage)?;
        PAYMENTS.save(deps.storage, id.into(), &PaymentState{
            Payment: p,
            paid: false,
            id,
        })?;
    };
    Ok(Response::new()
        .add_attribute("method", "instantiate"))
        //.add_attribute("count", msg.schedule))
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn execute(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, ContractError> {
    match msg {
        ExecuteMsg::Pay {} => execute_pay(deps, env),
    }
}

pub fn execute_pay(deps: DepsMut, env: Env) -> Result<Response,ContractError> {
    let to_be_paid:Vec<PaymentState> = PAYMENTS
        .range(deps.storage, None, None, Order::Ascending)
        .filter_map(|r| match r{
            Ok(r)=>Some(r.1),
            _ => None
        })
        .filter(|p|p.paid == false && p.Payment.time.is_expired(&env.block))
        .collect();

    // Get cosmos payment messages
    let payment_msgs:Vec<CosmosMsg> = to_be_paid.clone()
        .into_iter()
        .map(|p|get_payment_message(&p.Payment))
        .collect::<StdResult<Vec<CosmosMsg>>>()?;

    // Update payments to paid
    for p in to_be_paid.into_iter(){
        PAYMENTS.update(
            deps.storage,
            p.id.into(),
            |p| match p {
                Some(p)=>Ok(PaymentState{paid:true,..p}),
                None=>Err(ContractError::PaymentNotFound {}),
            }
        )?;
    }

    Ok(Response::new()
        .add_messages(payment_msgs))
        //.add_attribute("paid", to_be_paid))
}

pub fn get_payment_message(p:&Payment)->StdResult<CosmosMsg>{
    match p.token_address {
        Some(_)=>get_token_payment(p),
        None=>get_native_payment(p),
    }
}

pub fn get_token_payment(p:&Payment) -> StdResult<CosmosMsg> {
    let transfer_cw20_msg = Cw20ExecuteMsg::Transfer {
        recipient: p.recipient.to_string(),
        amount: p.amount,
    };

    let exec_cw20_transfer = WasmMsg::Execute {
        contract_addr: p.token_address.clone().unwrap().to_string(),
        msg: to_binary(&transfer_cw20_msg)?,
        funds: vec![],
    };

    Ok(exec_cw20_transfer.into())
}

pub fn get_native_payment(p:&Payment) -> StdResult<CosmosMsg> {
    println!("native_payment" );
    let transfer_bank_msg = cosmwasm_std::BankMsg::Send {
        to_address: p.recipient.clone().into_string(),
        amount: vec![Coin {
            denom: p.denom.clone(),
            amount: p.amount,
        }],
    };

    Ok(transfer_bank_msg.into())
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::GetPayments {} => to_binary(&query_payments(deps)?),
    }
}

fn query_payments(deps: Deps) -> StdResult<PaymentsResponse> {
    Ok(PaymentsResponse{
        payments: PAYMENTS
        .range(deps.storage, None, None, Order::Ascending)
        .filter_map(|p| match p {
            Ok(p)=>Some(p.1),
            Err(_)=> None
        })
        .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use cosmwasm_std::testing::{mock_dependencies, mock_env, mock_info, MockApi, MockStorage};
    use cosmwasm_std::{coins, from_binary, Empty, Addr, Uint128};
    use cw20::{Cw20Coin, Cw20Contract};
    use cw_multi_test::{next_block, App, BankKeeper, Contract, ContractWrapper, Executor, AppResponse};
    use std::borrow::Cow::Owned;
    use cw0::Expiration;
    use std::fmt::Error;

    const OWNER: &str = "owner0001";
    const FUNDER: &str = "funder";
    const PAYEE2: &str = "payee0002";
    const PAYEE3: &str = "payee0003";

    const NATIVE_TOKEN_DENOM: &str = "ujuno";
    const INITIAL_BALANCE: u128 = 2000000;

    pub fn contract_vest() -> Box<dyn Contract<Empty>> {
        let contract = ContractWrapper::new(
            crate::contract::execute,
            crate::contract::instantiate,
            crate::contract::query,
        );
        Box::new(contract)
    }

    pub fn contract_cw20() -> Box<dyn Contract<Empty>> {
        let contract = ContractWrapper::new(
            cw20_base::contract::execute,
            cw20_base::contract::instantiate,
            cw20_base::contract::query,
        );
        Box::new(contract)
    }

    fn mock_app() -> App {
        let env = mock_env();
        let api = MockApi::default();
        let bank = BankKeeper::new();

        App::new(api, env.block, bank, MockStorage::new())
    }

    // uploads code and returns address of cw20 contract
    fn instantiate_cw20(app: &mut App) -> Addr {
        let cw20_id = app.store_code(contract_cw20());
        let msg = cw20_base::msg::InstantiateMsg {
            name: String::from("Test"),
            symbol: String::from("TEST"),
            decimals: 6,
            initial_balances: vec![
                Cw20Coin {
                    address: OWNER.to_string(),
                    amount: Uint128::new(INITIAL_BALANCE),
                },
                Cw20Coin {
                    address: FUNDER.to_string(),
                    amount: Uint128::new(INITIAL_BALANCE),
                },
                Cw20Coin {
                    address: PAYEE2.to_string(),
                    amount: Uint128::new(INITIAL_BALANCE),
                },
                Cw20Coin {
                    address: PAYEE3.to_string(),
                    amount: Uint128::new(INITIAL_BALANCE * 2),
                },
            ],
            mint: None,
            marketing: None,
        };
        app.instantiate_contract(cw20_id, Addr::unchecked(OWNER), &msg, &[], "cw20", None)
            .unwrap()
    }

    fn instantiate_vest(
        app: &mut App,
        payments: Vec<Payment>
    ) -> Addr {
        let flex_id = app.store_code(contract_vest());
        let msg = crate::msg::InstantiateMsg {
            schedule: payments
        };
        app.instantiate_contract(flex_id, Addr::unchecked(OWNER), &msg, &[], "flex", None)
            .unwrap()
    }

    fn get_accounts(
    ) -> (Addr, Addr, Addr, Addr) {
        let owner: Addr = Addr::unchecked(OWNER);
        let funder: Addr = Addr::unchecked(FUNDER);
        let voter2: Addr = Addr::unchecked(PAYEE2);
        let voter3: Addr = Addr::unchecked(PAYEE3);

        (owner, funder, voter2, voter3)
    }

    fn fund_vest_contract(app: &mut App, vest: Addr, cw20: Addr, funder: Addr, amount: Uint128) {
        app.execute_contract(funder, cw20,&Cw20ExecuteMsg::Transfer { recipient: vest.to_string(), amount }, &vec![]);
    }

    #[test]
    fn proper_initialization() {
        let mut deps = mock_dependencies(&[]);

        let msg = InstantiateMsg { schedule: vec![]};
        let info = mock_info("creator", &coins(1000, "earth"));

        // we can just call .unwrap() to assert this was a success
        let res = instantiate(deps.as_mut(), mock_env(), info, msg).unwrap();
        assert_eq!(0, res.messages.len());

        // it worked, let's query the state
        let res = query(deps.as_ref(), mock_env(), QueryMsg::GetPayments {}).unwrap();
        let value: PaymentsResponse = from_binary(&res).unwrap();
        assert_eq!(0, value.payments.len());
    }

    #[test]
    fn get_payments() {
        let mut deps = mock_dependencies(&[]);

        let payment = Payment {
            recipient: Addr::unchecked(String::from("test")),
            amount: Uint128::new(1),
            denom: "".to_string(),
            token_address: None,
            time: Expiration::AtHeight(1)
        };
        let payment2 = payment.clone();
        let msg = InstantiateMsg {
            schedule: vec![payment.clone(), payment2]
        };
        let info = mock_info("creator", &coins(1000, "earth"));

        // we can just call .unwrap() to assert this was a success
        let res = instantiate(deps.as_mut(), mock_env(), info, msg).unwrap();
        assert_eq!(0, res.messages.len());

        // it worked, let's query the state
        let res = query(deps.as_ref(), mock_env(), QueryMsg::GetPayments {}).unwrap();
        let value: PaymentsResponse = from_binary(&res).unwrap();
        assert_eq!(2, value.payments.len());
    }

    #[test]
    fn proper_initialization_integration() {
        let mut app = mock_app();

        let (owner, funder, _payee2, _payee3) =
            get_accounts();

        let cw20_addr = instantiate_cw20(&mut app);
        let cw20 = Cw20Contract(cw20_addr.clone());

        let payments = vec![
            Payment{
                recipient: owner,
                amount: Uint128::new(1),
                denom: cw20_addr.to_string(),
                token_address: None,
                time: Default::default()
            }
        ];

        let vest_addr = instantiate_vest(&mut app,payments);
    }

    #[test]
    fn single_cw20_payment() {
        let mut app = mock_app();

        let (owner, funder, _payee2, _payee3) =
            get_accounts();

        let cw20_addr = instantiate_cw20(&mut app);
        let cw20 = Cw20Contract(cw20_addr.clone());

        assert_eq!(app.block_info().height,12345);

        let payments = vec![
            Payment{
                recipient: owner.clone(),
                amount: Uint128::new(1),
                denom: cw20_addr.to_string(),
                token_address: Some(cw20_addr.clone()),
                time: Expiration::AtHeight(1)
            }
        ];

        let vest_addr = instantiate_vest(&mut app,payments);

        fund_vest_contract(&mut app, vest_addr.clone(),cw20_addr.clone(),funder.clone(),Uint128::new(1));

        let initial_balance = cw20.balance(&app,owner.clone()).unwrap();
        let vest_balance = cw20.balance(&app,vest_addr.clone()).unwrap();
        assert_eq!(vest_balance, Uint128::new(1));

        // Payout vested tokens
        app.execute_contract(_payee3.clone(), vest_addr.clone(),&ExecuteMsg::Pay {},&vec![]).unwrap();

        assert_eq!(cw20.balance(&app, owner.clone()).unwrap(), Uint128::new(INITIAL_BALANCE+1));

        // Assert payment is not executed twice
        app.execute_contract(_payee3, vest_addr,&ExecuteMsg::Pay {},&vec![]).unwrap();

        assert_eq!(cw20.balance(&app, owner).unwrap(), Uint128::new(INITIAL_BALANCE+1));


    }

}

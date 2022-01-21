#[cfg(not(feature = "library"))]
use cosmwasm_std::entry_point;
use cosmwasm_std::{
    to_binary, Addr, Binary, Coin, CosmosMsg, Deps, DepsMut, Env, MessageInfo, Order, Response,
    StdResult, WasmMsg,
};

use crate::error::ContractError;
use crate::msg::{ConfigResponse, ExecuteMsg, InstantiateMsg, Payment, PaymentsResponse, QueryMsg};
use crate::state::{next_id, Config, PaymentState, CONFIG, PAYMENTS};
use cw20::Cw20ExecuteMsg;

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn instantiate(
    deps: DepsMut,
    _env: Env,
    _info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    let owner = deps.api.addr_validate(msg.owner.as_str())?;
    let config = Config { owner };
    CONFIG.save(deps.storage, &config)?;

    for p in msg.schedule.into_iter() {
        let id = next_id(deps.storage)?;
        PAYMENTS.save(
            deps.storage,
            id.into(),
            &PaymentState {
                payment: p,
                paid: false,
                stopped: false,
                id,
            },
        )?;
    }
    Ok(Response::new().add_attribute("method", "instantiate"))
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
        ExecuteMsg::UpdateConfig { owner } => execute_update_config(info, deps, owner),
        ExecuteMsg::StopPayment { id } => execute_stop_payment(info, deps, id),
        ExecuteMsg::AddPayments { schedule } => execute_add_payments(info, deps, schedule),
    }
}

pub fn execute_add_payments(
    info: MessageInfo,
    deps: DepsMut,
    schedule: Vec<Payment>,
) -> Result<Response, ContractError> {
    let config: Config = CONFIG.load(deps.storage)?;
    if info.sender != config.owner {
        return Err(ContractError::Unauthorized {});
    }

    for p in schedule.into_iter() {
        let id = next_id(deps.storage)?;
        PAYMENTS.save(
            deps.storage,
            id.into(),
            &PaymentState {
                payment: p,
                paid: false,
                stopped: false,
                id,
            },
        )?;
    }
    Ok(Response::new().add_attribute("method", "instantiate"))
}

pub fn execute_stop_payment(
    info: MessageInfo,
    deps: DepsMut,
    id: u64,
) -> Result<Response, ContractError> {
    let config: Config = CONFIG.load(deps.storage)?;
    if info.sender != config.owner {
        return Err(ContractError::Unauthorized {});
    }

    let payment = PAYMENTS
        .may_load(deps.storage, id.into())?
        .ok_or(ContractError::PaymentNotFound {})?;

    if payment.paid {
        return Err(ContractError::AlreadyPaid {});
    }

    if payment.stopped {
        return Err(ContractError::PaymentStopped {});
    }

    PAYMENTS.update(deps.storage, id.into(), |p| match p {
        Some(p) => Ok(PaymentState { stopped: true, ..p }),
        None => Err(ContractError::PaymentNotFound {}),
    })?;
    let refund_message = get_send_tokens_message(deps.as_ref(), &payment.payment, true)?;
    Ok(Response::new().add_message(refund_message))
}

pub fn execute_update_config(
    info: MessageInfo,
    deps: DepsMut,
    owner: Addr,
) -> Result<Response, ContractError> {
    let mut config: Config = CONFIG.load(deps.storage)?;
    if info.sender != config.owner {
        return Err(ContractError::Unauthorized {});
    }

    config.owner = owner.clone();

    CONFIG.save(deps.storage, &config)?;
    Ok(Response::new().add_attribute("owner", owner.to_string()))
}

pub fn execute_pay(deps: DepsMut, env: Env) -> Result<Response, ContractError> {
    let to_be_paid: Vec<PaymentState> = PAYMENTS
        .range(deps.storage, None, None, Order::Ascending)
        .filter_map(|r| match r {
            Ok(r) => Some(r.1),
            _ => None,
        })
        .filter(|p| !p.stopped && !p.paid && p.payment.time.is_expired(&env.block))
        .collect();

    // Get cosmos payment messages
    let payment_msgs: Vec<CosmosMsg> = to_be_paid
        .clone()
        .into_iter()
        .map(|p| get_send_tokens_message(deps.as_ref(), &p.payment, false))
        .collect::<StdResult<Vec<CosmosMsg>>>()?;

    // Update payments to paid
    for p in to_be_paid.into_iter() {
        PAYMENTS.update(deps.storage, p.id.into(), |p| match p {
            Some(p) => Ok(PaymentState { paid: true, ..p }),
            None => Err(ContractError::PaymentNotFound {}),
        })?;
    }

    Ok(Response::new().add_messages(payment_msgs))
}

pub fn get_send_tokens_message(deps: Deps, p: &Payment, refund: bool) -> StdResult<CosmosMsg> {
    let mut recipient = p.recipient.to_string();

    if refund {
        let config: Config = CONFIG.load(deps.storage)?;
        recipient = config.owner.to_string();
    }

    match p.token_address {
        Some(_) => Ok(WasmMsg::Execute {
            contract_addr: p.token_address.clone().unwrap().to_string(),
            msg: to_binary(&Cw20ExecuteMsg::Transfer {
                recipient,
                amount: p.amount,
            })?,
            funds: vec![],
        }
        .into()),
        None => Ok(cosmwasm_std::BankMsg::Send {
            to_address: recipient,
            amount: vec![Coin {
                denom: p.denom.clone(),
                amount: p.amount,
            }],
        }
        .into()),
    }
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> StdResult<Binary> {
    match msg {
        QueryMsg::GetPayments {} => to_binary(&query_payments(deps)),
        QueryMsg::GetConfig {} => to_binary(&query_config(deps)?),
    }
}

// Support range queries!!
fn query_payments(deps: Deps) -> PaymentsResponse {
    PaymentsResponse {
        payments: PAYMENTS
            .range(deps.storage, None, None, Order::Ascending)
            .filter_map(|p| match p {
                Ok(p) => Some(p.1),
                Err(_) => None,
            })
            .collect(),
    }
}

fn query_config(deps: Deps) -> StdResult<ConfigResponse> {
    let config: Config = CONFIG.load(deps.storage)?;
    Ok(ConfigResponse {
        owner: config.owner,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use cosmwasm_std::testing::{mock_dependencies, mock_env, mock_info, MockApi, MockStorage};
    use cosmwasm_std::{coin, coins, from_binary, Addr, Empty, Uint128};
    use cw0::Expiration;
    use cw20::{Cw20Coin, Cw20Contract};
    use cw_multi_test::{
        next_block, App, AppResponse, BankKeeper, Contract, ContractWrapper, Executor,
    };

    const OWNER: &str = "owner0001";
    const FUNDER: &str = "funder";
    const PAYEE2: &str = "payee0002";
    const PAYEE3: &str = "payee0003";

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

    fn instantiate_vest(app: &mut App, payments: Vec<Payment>) -> Addr {
        let flex_id = app.store_code(contract_vest());
        let msg = crate::msg::InstantiateMsg {
            owner: Addr::unchecked(OWNER),
            schedule: payments,
        };
        app.instantiate_contract(flex_id, Addr::unchecked(OWNER), &msg, &[], "flex", None)
            .unwrap()
    }

    fn get_accounts() -> (Addr, Addr, Addr, Addr) {
        let owner: Addr = Addr::unchecked(OWNER);
        let funder: Addr = Addr::unchecked(FUNDER);
        let voter2: Addr = Addr::unchecked(PAYEE2);
        let voter3: Addr = Addr::unchecked(PAYEE3);

        (owner, funder, voter2, voter3)
    }

    fn fund_vest_contract(
        app: &mut App,
        vest: Addr,
        cw20: Addr,
        funder: Addr,
        amount: Uint128,
    ) -> AppResponse {
        app.execute_contract(
            funder,
            cw20,
            &Cw20ExecuteMsg::Transfer {
                recipient: vest.to_string(),
                amount,
            },
            &[],
        )
        .unwrap()
    }

    #[test]
    fn proper_initialization() {
        let mut deps = mock_dependencies(&[]);

        let msg = InstantiateMsg {
            owner: Addr::unchecked(OWNER),
            schedule: vec![],
        };
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
    fn get_config() {
        let mut deps = mock_dependencies(&[]);

        let payment = Payment {
            recipient: Addr::unchecked(String::from("test")),
            amount: Uint128::new(1),
            denom: "".to_string(),
            token_address: None,
            time: Expiration::AtHeight(1),
        };
        let payment2 = payment.clone();
        let msg = InstantiateMsg {
            owner: Addr::unchecked(OWNER),
            schedule: vec![payment, payment2],
        };
        let info = mock_info("creator", &coins(1000, "earth"));

        // we can just call .unwrap() to assert this was a success
        let res = instantiate(deps.as_mut(), mock_env(), info, msg).unwrap();
        assert_eq!(0, res.messages.len());

        // it worked, let's query the state
        let res = query(deps.as_ref(), mock_env(), QueryMsg::GetConfig {}).unwrap();
        let value: ConfigResponse = from_binary(&res).unwrap();
        assert_eq!(value.owner, Addr::unchecked(OWNER));
    }

    #[test]
    fn update_config() {
        let mut deps = mock_dependencies(&[]);

        let payment = Payment {
            recipient: Addr::unchecked(String::from("test")),
            amount: Uint128::new(1),
            denom: "".to_string(),
            token_address: None,
            time: Expiration::AtHeight(1),
        };
        let payment2 = payment.clone();
        let msg = InstantiateMsg {
            owner: Addr::unchecked(OWNER),
            schedule: vec![payment, payment2],
        };
        let info = mock_info(OWNER, &coins(1000, "earth"));

        let res = instantiate(deps.as_mut(), mock_env(), info.clone(), msg).unwrap();
        assert_eq!(0, res.messages.len());

        let msg = ExecuteMsg::UpdateConfig {
            owner: Addr::unchecked("owner2"),
        };
        execute(deps.as_mut(), mock_env(), info.clone(), msg).unwrap();

        let res = query(deps.as_ref(), mock_env(), QueryMsg::GetConfig {}).unwrap();
        let value: ConfigResponse = from_binary(&res).unwrap();
        assert_eq!(value.owner, Addr::unchecked("owner2"));

        // try updating with invalid owner

        let msg = ExecuteMsg::UpdateConfig {
            owner: Addr::unchecked(OWNER),
        };
        let err = execute(deps.as_mut(), mock_env(), info, msg).unwrap_err();

        assert_eq!(err, ContractError::Unauthorized {});
    }

    #[test]
    fn get_payments() {
        let mut deps = mock_dependencies(&[]);

        let payment = Payment {
            recipient: Addr::unchecked(String::from("test")),
            amount: Uint128::new(1),
            denom: "".to_string(),
            token_address: None,
            time: Expiration::AtHeight(1),
        };
        let payment2 = payment.clone();
        let msg = InstantiateMsg {
            owner: Addr::unchecked(OWNER),
            schedule: vec![payment, payment2],
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

        let (owner, _funder, _payee2, _payee3) = get_accounts();

        let cw20_addr = instantiate_cw20(&mut app);

        let payments = vec![Payment {
            recipient: owner,
            amount: Uint128::new(1),
            denom: cw20_addr.to_string(),
            token_address: None,
            time: Default::default(),
        }];

        instantiate_vest(&mut app, payments);
    }

    #[test]
    fn single_cw20_payment() {
        let mut app = mock_app();

        let (owner, funder, _payee2, _payee3) = get_accounts();

        let cw20_addr = instantiate_cw20(&mut app);
        let cw20 = Cw20Contract(cw20_addr.clone());

        let payments = vec![Payment {
            recipient: owner.clone(),
            amount: Uint128::new(1),
            denom: cw20_addr.to_string(),
            token_address: Some(cw20_addr.clone()),
            time: Expiration::AtHeight(1),
        }];

        let vest_addr = instantiate_vest(&mut app, payments);

        fund_vest_contract(
            &mut app,
            vest_addr.clone(),
            cw20_addr,
            funder,
            Uint128::new(1),
        );

        let owner_balance = |app: &App<Empty>| cw20.balance(app, owner.clone()).unwrap().u128();
        let initial_balance = owner_balance(&app);
        let vest_balance = cw20.balance(&app, vest_addr.clone()).unwrap().u128();
        assert_eq!(vest_balance, 1);

        // Payout vested tokens
        app.execute_contract(_payee3.clone(), vest_addr.clone(), &ExecuteMsg::Pay {}, &[])
            .unwrap();
        assert_eq!(owner_balance(&app), initial_balance + 1);

        // Assert payment is not executed twice
        app.execute_contract(_payee3, vest_addr, &ExecuteMsg::Pay {}, &[])
            .unwrap();
        assert_eq!(owner_balance(&app), initial_balance + 1);
    }

    #[test]
    fn multiple_cw20_payment() {
        let mut app = mock_app();

        let (owner, funder, _payee2, _payee3) = get_accounts();

        let cw20_addr = instantiate_cw20(&mut app);
        let cw20 = Cw20Contract(cw20_addr.clone());

        let current_height = app.block_info().height;

        let payments = vec![
            Payment {
                recipient: owner.clone(),
                amount: Uint128::new(1),
                denom: cw20_addr.to_string(),
                token_address: Some(cw20_addr.clone()),
                time: Expiration::AtHeight(current_height + 1),
            },
            Payment {
                recipient: owner.clone(),
                amount: Uint128::new(2),
                denom: cw20_addr.to_string(),
                token_address: Some(cw20_addr.clone()),
                time: Expiration::AtHeight(current_height + 2),
            },
            Payment {
                recipient: owner.clone(),
                amount: Uint128::new(2),
                denom: cw20_addr.to_string(),
                token_address: Some(cw20_addr.clone()),
                time: Expiration::AtHeight(current_height + 2),
            },
            Payment {
                recipient: owner.clone(),
                amount: Uint128::new(5),
                denom: cw20_addr.to_string(),
                token_address: Some(cw20_addr.clone()),
                time: Expiration::AtHeight(current_height + 3),
            },
        ];

        let vest_addr = instantiate_vest(&mut app, payments);

        fund_vest_contract(
            &mut app,
            vest_addr.clone(),
            cw20_addr,
            funder,
            Uint128::new(10),
        );

        let owner_balance = |app: &App<Empty>| cw20.balance(app, owner.clone()).unwrap().u128();
        let initial_balance = owner_balance(&app);
        let vest_balance = cw20.balance(&app, vest_addr.clone()).unwrap().u128();
        assert_eq!(vest_balance, 10);

        // Payout vested tokens
        app.execute_contract(_payee3.clone(), vest_addr.clone(), &ExecuteMsg::Pay {}, &[])
            .unwrap();

        assert_eq!(owner_balance(&app), initial_balance);

        // Update block and pay first payment
        app.update_block(next_block);
        app.execute_contract(_payee3.clone(), vest_addr.clone(), &ExecuteMsg::Pay {}, &[])
            .unwrap();
        assert_eq!(owner_balance(&app), initial_balance + 1);

        // Check second call does not make more payments
        app.execute_contract(_payee3.clone(), vest_addr.clone(), &ExecuteMsg::Pay {}, &[])
            .unwrap();
        assert_eq!(owner_balance(&app), initial_balance + 1);

        // Update block and make 2nd and 3rd payments
        app.update_block(next_block);
        app.execute_contract(_payee3.clone(), vest_addr.clone(), &ExecuteMsg::Pay {}, &[])
            .unwrap();
        assert_eq!(owner_balance(&app), initial_balance + 5);

        // Check second call does not make more payments
        app.execute_contract(_payee3.clone(), vest_addr.clone(), &ExecuteMsg::Pay {}, &[])
            .unwrap();
        assert_eq!(owner_balance(&app), initial_balance + 5);

        // Update block and make 4th payments
        app.update_block(next_block);
        app.execute_contract(_payee3.clone(), vest_addr.clone(), &ExecuteMsg::Pay {}, &[])
            .unwrap();
        assert_eq!(owner_balance(&app), initial_balance + 10);

        // Check second call does not make more payments
        app.execute_contract(_payee3, vest_addr.clone(), &ExecuteMsg::Pay {}, &[])
            .unwrap();
        assert_eq!(owner_balance(&app), initial_balance + 10);

        // Assert contract has spent all funds
        let vest_balance = cw20.balance(&app, vest_addr).unwrap().u128();
        assert_eq!(vest_balance, 0);
    }

    #[test]
    fn single_native_payment() {
        let mut app = mock_app();

        let (owner, _funder, _payee2, _payee3) = get_accounts();

        let denom = String::from("ujuno");
        let payments = vec![Payment {
            recipient: owner.clone(),
            amount: Uint128::new(1),
            denom: denom.clone(),
            token_address: None,
            time: Expiration::AtHeight(1),
        }];

        let vest_addr = instantiate_vest(&mut app, payments);

        // Fund vest contract
        app.init_bank_balance(&vest_addr, vec![coin(1, denom.clone())])
            .unwrap();

        let owner_balance = |app: &App<Empty>| {
            app.wrap()
                .query_balance(owner.clone(), denom.clone())
                .unwrap()
                .amount
                .u128()
        };
        let initial_balance = owner_balance(&app);

        // Payout vested tokens
        app.execute_contract(_payee3.clone(), vest_addr.clone(), &ExecuteMsg::Pay {}, &[])
            .unwrap();
        assert_eq!(owner_balance(&app), initial_balance + 1);

        // Assert payment is not executed twice
        app.execute_contract(_payee3, vest_addr, &ExecuteMsg::Pay {}, &[])
            .unwrap();
        assert_eq!(owner_balance(&app), initial_balance + 1);
    }

    #[test]
    fn add_payments() {
        let mut app = mock_app();

        let (owner, _funder, _payee2, _payee3) = get_accounts();

        instantiate_cw20(&mut app);

        let current_height = app.block_info().height;

        let denom = String::from("ujuno");
        let payments = vec![Payment {
            recipient: owner.clone(),
            amount: Uint128::new(1),
            denom: denom.clone(),
            token_address: None,
            time: Expiration::AtHeight(current_height + 1),
        }];

        let vest_addr = instantiate_vest(&mut app, payments);

        // Fund vest contract
        app.init_bank_balance(&vest_addr, vec![coin(10, denom.clone())])
            .unwrap();

        let owner_balance = |app: &App<Empty>| {
            app.wrap()
                .query_balance(owner.clone(), denom.clone())
                .unwrap()
                .amount
                .u128()
        };
        let initial_balance = owner_balance(&app);

        // Payout vested tokens
        app.execute_contract(_payee3.clone(), vest_addr.clone(), &ExecuteMsg::Pay {}, &[])
            .unwrap();

        assert_eq!(owner_balance(&app), initial_balance);

        // Update block and pay first payment
        app.update_block(next_block);
        app.execute_contract(_payee3.clone(), vest_addr.clone(), &ExecuteMsg::Pay {}, &[])
            .unwrap();
        assert_eq!(owner_balance(&app), initial_balance + 1);

        let initial_balance = owner_balance(&app);
        let new_payments = vec![Payment {
            recipient: owner.clone(),
            amount: Uint128::new(2),
            denom: denom.clone(),
            token_address: None,
            time: Expiration::AtHeight(current_height + 1),
        }];
        // Add additional payment and update block
        app.execute_contract(
            owner.clone(),
            vest_addr.clone(),
            &ExecuteMsg::AddPayments {
                schedule: new_payments,
            },
            &[],
        )
        .unwrap();
        app.update_block(next_block);
        app.execute_contract(_payee3, vest_addr.clone(), &ExecuteMsg::Pay {}, &[])
            .unwrap();
        assert_eq!(owner_balance(&app), initial_balance + 2);
    }

    #[test]
    fn stop_payments() {
        let mut deps = mock_dependencies(&[]);

        let denom = String::from("ujuno");
        let payment = Payment {
            recipient: Addr::unchecked(String::from("test")),
            amount: Uint128::new(1),
            denom: denom.clone(),
            token_address: None,
            time: Expiration::AtHeight(1),
        };
        let payment2 = Payment {
            recipient: Addr::unchecked(String::from("test")),
            amount: Uint128::new(2),
            denom: denom.clone(),
            token_address: None,
            time: Expiration::AtHeight(1),
        };
        let msg = InstantiateMsg {
            owner: Addr::unchecked(OWNER),
            schedule: vec![payment, payment2],
        };

        let info = mock_info(OWNER, &coins(1000, denom.clone()));
        let res = instantiate(deps.as_mut(), mock_env(), info, msg).unwrap();
        assert_eq!(0, res.messages.len());

        // Try stopping payment with invalid sender
        let msg = ExecuteMsg::StopPayment { id: 1 };
        let info = mock_info("fakeOwner", &coins(0, denom.clone()));
        let err = execute(deps.as_mut(), mock_env(), info, msg.clone()).unwrap_err();
        assert_eq!(err, ContractError::Unauthorized {});

        // Stop payment using correct owner
        let info = mock_info(OWNER, &coins(0, denom.clone()));
        let res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();

        // Check if payment amount is refunded
        assert_eq!(
            res.messages[0],
            cosmwasm_std::SubMsg::new(cosmwasm_std::BankMsg::Send {
                to_address: OWNER.to_string(),
                amount: vec![Coin {
                    denom: denom.clone(),
                    amount: Uint128::new(1),
                }],
            })
        );

        // Execute remaining payments
        let msg = ExecuteMsg::Pay {};
        let info = mock_info(OWNER, &coins(0, denom.clone()));
        let res = execute(deps.as_mut(), mock_env(), info, msg).unwrap();
        assert_eq!(res.messages.len(), 1);
        assert_eq!(
            res.messages[0],
            cosmwasm_std::SubMsg::new(cosmwasm_std::BankMsg::Send {
                to_address: String::from("test"),
                amount: vec![Coin {
                    denom,
                    amount: Uint128::new(2),
                }],
            })
        );
    }

    #[test]
    fn multiple_native_payment() {
        let mut app = mock_app();

        let (owner, _funder, _payee2, _payee3) = get_accounts();

        instantiate_cw20(&mut app);

        let current_height = app.block_info().height;

        let denom = String::from("ujuno");
        let payments = vec![
            Payment {
                recipient: owner.clone(),
                amount: Uint128::new(1),
                denom: denom.clone(),
                token_address: None,
                time: Expiration::AtHeight(current_height + 1),
            },
            Payment {
                recipient: owner.clone(),
                amount: Uint128::new(2),
                denom: denom.clone(),
                token_address: None,
                time: Expiration::AtHeight(current_height + 2),
            },
            Payment {
                recipient: owner.clone(),
                amount: Uint128::new(2),
                denom: denom.clone(),
                token_address: None,
                time: Expiration::AtHeight(current_height + 2),
            },
            Payment {
                recipient: owner.clone(),
                amount: Uint128::new(5),
                denom: denom.clone(),
                token_address: None,
                time: Expiration::AtHeight(current_height + 3),
            },
        ];

        let vest_addr = instantiate_vest(&mut app, payments);

        // Fund vest contract
        app.init_bank_balance(&vest_addr, vec![coin(10, denom.clone())])
            .unwrap();

        let owner_balance = |app: &App<Empty>| {
            app.wrap()
                .query_balance(owner.clone(), denom.clone())
                .unwrap()
                .amount
                .u128()
        };
        let initial_balance = owner_balance(&app);

        // Payout vested tokens
        app.execute_contract(_payee3.clone(), vest_addr.clone(), &ExecuteMsg::Pay {}, &[])
            .unwrap();

        assert_eq!(owner_balance(&app), initial_balance);

        // Update block and pay first payment
        app.update_block(next_block);
        app.execute_contract(_payee3.clone(), vest_addr.clone(), &ExecuteMsg::Pay {}, &[])
            .unwrap();
        assert_eq!(owner_balance(&app), initial_balance + 1);

        // Check second call does not make more payments
        app.execute_contract(_payee3.clone(), vest_addr.clone(), &ExecuteMsg::Pay {}, &[])
            .unwrap();
        assert_eq!(owner_balance(&app), initial_balance + 1);

        // Update block and make 2nd and 3rd payments
        app.update_block(next_block);
        app.execute_contract(_payee3.clone(), vest_addr.clone(), &ExecuteMsg::Pay {}, &[])
            .unwrap();
        assert_eq!(owner_balance(&app), initial_balance + 5);

        // Check second call does not make more payments
        app.execute_contract(_payee3.clone(), vest_addr.clone(), &ExecuteMsg::Pay {}, &[])
            .unwrap();
        assert_eq!(owner_balance(&app), initial_balance + 5);

        // Update block and make 4th payments
        app.update_block(next_block);
        app.execute_contract(_payee3.clone(), vest_addr.clone(), &ExecuteMsg::Pay {}, &[])
            .unwrap();
        assert_eq!(owner_balance(&app), initial_balance + 10);

        // Check second call does not make more payments
        app.execute_contract(_payee3, vest_addr.clone(), &ExecuteMsg::Pay {}, &[])
            .unwrap();
        assert_eq!(owner_balance(&app), initial_balance + 10);
    }

    #[test]
    fn native_and_token_payments() {
        let mut app = mock_app();

        let (owner, funder, _payee2, _payee3) = get_accounts();

        let cw20_addr = instantiate_cw20(&mut app);
        let cw20 = Cw20Contract(cw20_addr.clone());

        let current_height = app.block_info().height;

        let denom = String::from("ujuno");
        let payments = vec![
            Payment {
                recipient: owner.clone(),
                amount: Uint128::new(1),
                denom: denom.clone(),
                token_address: None,
                time: Expiration::AtHeight(current_height + 1),
            },
            Payment {
                recipient: owner.clone(),
                amount: Uint128::new(2),
                denom: String::new(),
                token_address: Some(cw20_addr.clone()),
                time: Expiration::AtHeight(current_height + 2),
            },
            Payment {
                recipient: owner.clone(),
                amount: Uint128::new(2),
                denom: denom.clone(),
                token_address: None,
                time: Expiration::AtHeight(current_height + 2),
            },
            Payment {
                recipient: owner.clone(),
                amount: Uint128::new(5),
                denom: String::new(),
                token_address: Some(cw20_addr.clone()),
                time: Expiration::AtHeight(current_height + 3),
            },
        ];

        let vest_addr = instantiate_vest(&mut app, payments);

        // Fund vest contract
        app.init_bank_balance(&vest_addr, vec![coin(3, denom.clone())])
            .unwrap();
        fund_vest_contract(
            &mut app,
            vest_addr.clone(),
            cw20_addr,
            funder,
            Uint128::new(7),
        );

        let owner_balance_cw20 =
            |app: &App<Empty>| cw20.balance(app, owner.clone()).unwrap().u128();
        let owner_balance_juno = |app: &App<Empty>| {
            app.wrap()
                .query_balance(owner.clone(), denom.clone())
                .unwrap()
                .amount
                .u128()
        };
        let initial_balance_cw20 = owner_balance_cw20(&app);
        let initial_balance_juno = owner_balance_juno(&app);

        // Payout vested tokens
        app.execute_contract(_payee3.clone(), vest_addr.clone(), &ExecuteMsg::Pay {}, &[])
            .unwrap();

        assert_eq!(owner_balance_cw20(&app), initial_balance_cw20);
        assert_eq!(owner_balance_juno(&app), initial_balance_juno);

        // Update block and pay first payment
        app.update_block(next_block);
        app.execute_contract(_payee3.clone(), vest_addr.clone(), &ExecuteMsg::Pay {}, &[])
            .unwrap();
        assert_eq!(owner_balance_cw20(&app), initial_balance_cw20);
        assert_eq!(owner_balance_juno(&app), initial_balance_juno + 1);

        // Check second call does not make more payments
        app.execute_contract(_payee3.clone(), vest_addr.clone(), &ExecuteMsg::Pay {}, &[])
            .unwrap();
        assert_eq!(owner_balance_cw20(&app), initial_balance_cw20);
        assert_eq!(owner_balance_juno(&app), initial_balance_juno + 1);

        // Update block and make 2nd and 3rd payments
        app.update_block(next_block);
        app.execute_contract(_payee3.clone(), vest_addr.clone(), &ExecuteMsg::Pay {}, &[])
            .unwrap();
        assert_eq!(owner_balance_cw20(&app), initial_balance_cw20 + 2);
        assert_eq!(owner_balance_juno(&app), initial_balance_juno + 3);

        // Check second call does not make more payments
        app.execute_contract(_payee3.clone(), vest_addr.clone(), &ExecuteMsg::Pay {}, &[])
            .unwrap();
        assert_eq!(owner_balance_cw20(&app), initial_balance_cw20 + 2);
        assert_eq!(owner_balance_juno(&app), initial_balance_juno + 3);

        // Update block and make 4th payments
        app.update_block(next_block);
        app.execute_contract(_payee3.clone(), vest_addr.clone(), &ExecuteMsg::Pay {}, &[])
            .unwrap();
        assert_eq!(owner_balance_cw20(&app), initial_balance_cw20 + 7);
        assert_eq!(owner_balance_juno(&app), initial_balance_juno + 3);

        // Check second call does not make more payments
        app.execute_contract(_payee3, vest_addr.clone(), &ExecuteMsg::Pay {}, &[])
            .unwrap();
        assert_eq!(owner_balance_cw20(&app), initial_balance_cw20 + 7);
        assert_eq!(owner_balance_juno(&app), initial_balance_juno + 3);
    }
}

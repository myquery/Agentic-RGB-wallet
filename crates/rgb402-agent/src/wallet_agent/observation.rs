//! Provider-facing projection; application approval views retain the full bound details.
use super::{OutcomeStatus, ToolOutput};
use serde_json::{json, Value};
pub const MAX_OBSERVATION_BYTES: usize = 8192;

pub fn model_observation(output: &ToolOutput) -> Value {
    let mut value = serde_json::to_value(output)
        .unwrap_or_else(|_| json!({"type":"error","code":"observation_unavailable"}));
    match output {
        ToolOutput::BtcPlan { plan } => {
            value["plan"]["recipient"]
                .as_object_mut()
                .unwrap()
                .remove("service_url");
            value["payment_authorized"] = json!(false);
            value["next_allowed_actions"] = if matches!(
                plan.policy,
                rgb402_core::wallet::PolicyDecision::Deny { .. }
            ) {
                json!([])
            } else {
                json!(["await_application_authorization"])
            };
        }
        ToolOutput::Invoice { .. } => {
            value["request"].as_object_mut().unwrap().remove("invoice");
            value["next_allowed_actions"] = json!(["prepare_payment"]);
        }
        ToolOutput::Plan { plan } if plan.recipient.is_some() => {
            value = json!({
                "type": "plan", "status": "prepared",
                "plan": {
                    "plan_id": plan.plan_id, "recipient": plan.recipient,
                    "asset_id": plan.request.asset_id, "amount": plan.request.amount.to_string(),
                    "available_balance": plan.available_balance, "policy": plan.policy,
                    "application_confirmation_required": plan.application_confirmation_required,
                },
                "payment_authorized": !plan.application_confirmation_required
                    && matches!(plan.policy, rgb402_core::wallet::PolicyDecision::Allow),
                "next_allowed_actions": if matches!(plan.policy, rgb402_core::wallet::PolicyDecision::Deny { .. }) {
                    json!([])
                } else if plan.application_confirmation_required {
                    json!(["await_application_authorization"])
                } else {
                    json!(["execute_bound_plan"])
                }
            });
            if let rgb402_core::wallet::PolicyDecision::Deny { reason } = &plan.policy {
                value["status"] = json!("denied");
                value["code"] = json!(if reason == "insufficient outbound RGB balance" {
                    "insufficient_balance"
                } else {
                    "policy_denied"
                });
            }
        }
        ToolOutput::Plan { plan } => {
            value["plan"]["request"]
                .as_object_mut()
                .unwrap()
                .remove("invoice");
            value["next_allowed_actions"] = if matches!(
                plan.policy,
                rgb402_core::wallet::PolicyDecision::Deny { .. }
            ) {
                json!([])
            } else {
                json!(["await_application_authorization"])
            };
        }
        ToolOutput::Payment { status, .. } | ToolOutput::BtcPayment { status, .. } => {
            value["next_allowed_actions"] = match status {
                OutcomeStatus::Pending | OutcomeStatus::Uncertain => json!(["query_status"]),
                _ => json!([]),
            };
            value["evidence_source"] = json!("wallet_node");
        }
        ToolOutput::Machine { purchase } => {
            value["next_allowed_actions"] = match purchase.resource_status.as_str() {
                "approval_required" => json!(["await_application_authorization"]),
                "payment_unresolved" | "paid_resource_unavailable" => json!(["fetch_same_url"]),
                _ => json!([]),
            };
            if purchase.payment_status.is_some() {
                value["retry_submits_payment"] = json!(false);
            }
            value["evidence_source"] = json!("payment_service_and_merchant");
            // The UI retains the entire bounded resource; model context gets at most 4 KiB.
            if serde_json::to_vec(&purchase.resource).map_or(true, |b| b.len() > 4096) {
                value["purchase"]["resource"] = Value::Null;
                value["purchase"]["resource_omitted"] = json!(
                    "Resource exceeds model observation budget; retained in application view"
                );
            }
        }
        ToolOutput::Error { code, .. } => {
            value["retryability"] = json!(match code.as_str() {
                "duplicate" => "status_or_existing_resource_only",
                "wallet_failure" => "inspect_authoritative_state_first",
                "approval_required" => "application_authorization_required",
                _ => "correct_input_or_stop",
            });
            value["automatic_retry_allowed"] = json!(false);
        }
        _ => (),
    }
    if serde_json::to_vec(&value).map_or(true, |b| b.len() > MAX_OBSERVATION_BYTES) {
        return json!({"type":"error","code":"observation_too_large","message":"Application result exceeds model context budget; inspect the application view.","retryability":"do_not_repeat_economic_execution"});
    }
    value
}
/// Disclose the exact task summary, never its full trace or private contract.
pub fn with_task(
    output: &ToolOutput,
    task: Option<&rgb402_payment::harness::TaskSnapshot>,
) -> Value {
    use rgb402_payment::harness::{Authorization, State, TaskKind};
    let mut value = model_observation(output);
    if let Some(task) = task {
        if matches!(output, ToolOutput::Plan { plan } if plan.recipient.is_some()) {
            value["payment_authorized"] = json!(task.state == State::Authorized);
        }
        value["task"] = json!({"economic_action_id":task.economic_action_id,"kind":task.kind,"state":task.state,"authorization":task.authorization,"submission_may_have_occurred":task.submission_may_have_occurred});
        value["next_allowed_actions"] = match task.state {
            State::Authorized
                if task.kind == TaskKind::RgbPayment
                    && (task.authorization == Some(Authorization::Human)
                        || matches!(output, ToolOutput::Plan { plan } if plan.recipient.is_some() && !plan.application_confirmation_required)) =>
            {
                json!(["execute_bound_plan"])
            }
            State::Authorized if task.kind == TaskKind::L402Resource => json!(["fetch_same_url"]),
            State::Authorized | State::AwaitingAuthorization => {
                json!(["await_application_authorization"])
            }
            State::Submitted | State::Uncertain | State::Settled | State::ProofReady
                if task.kind == TaskKind::L402Resource =>
            {
                json!(["fetch_same_url"])
            }
            State::Submitted | State::Uncertain => json!(["query_status"]),
            _ => json!([]),
        };
    }
    if serde_json::to_vec(&value).map_or(true, |b| b.len() > MAX_OBSERVATION_BYTES) {
        return json!({"type":"error","code":"observation_too_large","retryability":"do_not_repeat_economic_execution"});
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wallet_agent::PlanView;
    use rgb402_core::{
        wallet::{PaymentRequest, PolicyDecision},
        AssetId, PaymentId,
    };
    #[test]
    fn invoice_is_not_duplicated_in_model_plan_context() {
        let output = ToolOutput::Plan {
            plan: PlanView {
                merchant_order: None,
                plan_id: "bound-plan".into(),
                request: PaymentRequest {
                    asset_id: AssetId::new("rgb:demo").unwrap(),
                    amount: 5,
                    invoice: "lnbcrt".repeat(1000),
                    payment_hash: PaymentId::new("hash").unwrap(),
                    expires_at: 999,
                    network: "Regtest".into(),
                    carrier_msat: 3000000,
                },
                available_balance: "100".into(),
                policy: PolicyDecision::RequireApproval {
                    reason: "limit".into(),
                },
                application_confirmation_required: true,
                recipient: None,
            },
        };
        let before = serde_json::to_vec(&output).unwrap().len();
        let task = rgb402_payment::harness::TaskSnapshot {
            economic_action_id: "a".repeat(64),
            kind: rgb402_payment::harness::TaskKind::RgbPayment,
            state: rgb402_payment::harness::State::AwaitingAuthorization,
            authorization: None,
            observed_submission_attempts: 0,
            submission_may_have_occurred: false,
            trace: vec![],
        };
        let projected = with_task(&output, Some(&task));
        let after = serde_json::to_vec(&projected).unwrap().len();
        assert!(after < before / 5);
        assert!(projected["plan"]["request"].get("invoice").is_none());
        assert_eq!(projected["plan"]["request"]["amount"], 5);
        assert_eq!(
            projected["next_allowed_actions"],
            json!(["await_application_authorization"])
        );
        println!("model plan JSON bytes: {before} -> {after}");
    }
    #[test]
    fn oversized_and_failed_observations_are_bounded_and_structured() {
        let output = ToolOutput::Error {
            code: "wallet_failure".into(),
            message: "x".repeat(65536),
        };
        let value = model_observation(&output);
        assert_eq!(value["code"], "observation_too_large");
        assert!(value.to_string().len() < MAX_OBSERVATION_BYTES);
        let value = model_observation(&ToolOutput::Error {
            code: "duplicate".into(),
            message: "Already reserved".into(),
        });
        assert_eq!(value["retryability"], "status_or_existing_resource_only");
    }
}

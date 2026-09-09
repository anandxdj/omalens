use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use omacam_core::{
    CaptureState, ConnectionState, OutputState, SessionPolicy, SessionSnapshot, TrustState,
};
use serde::Serialize;
use tokio::sync::{mpsc, watch};

const MAX_OPERATIONS: usize = 32;

#[derive(Debug)]
pub(crate) enum ServiceIntent {
    RequestStart { operation_id: String },
    Stop { operation_id: String },
    ForgetPeer { operation_id: String },
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum OperationState {
    Pending,
    Succeeded,
    Failed,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct OperationSnapshot {
    pub(crate) id: String,
    pub(crate) intent: &'static str,
    pub(crate) state: OperationState,
    pub(crate) error_code: Option<&'static str>,
    pub(crate) message: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct RuntimeSnapshot {
    pub(crate) revision: u64,
    pub(crate) state: SessionSnapshot,
    pub(crate) operations: Vec<OperationSnapshot>,
    pub(crate) last_error: Option<OperationSnapshot>,
}

#[derive(Debug)]
struct RuntimeInner {
    revision: u64,
    state: SessionSnapshot,
    operations: VecDeque<OperationSnapshot>,
    last_error: Option<OperationSnapshot>,
}

#[derive(Clone, Debug)]
pub(crate) struct ServiceRuntime {
    inner: Arc<Mutex<RuntimeInner>>,
    intent_tx: mpsc::Sender<ServiceIntent>,
    revision_tx: watch::Sender<u64>,
}

impl ServiceRuntime {
    pub(crate) fn new(
        policy: &SessionPolicy,
        _trust_path: std::path::PathBuf,
    ) -> (Self, mpsc::Receiver<ServiceIntent>) {
        let (intent_tx, intent_rx) = mpsc::channel(16);
        let (revision_tx, _) = watch::channel(1);
        (
            Self {
                inner: Arc::new(Mutex::new(RuntimeInner {
                    revision: 1,
                    state: policy.snapshot(),
                    operations: VecDeque::new(),
                    last_error: None,
                })),
                intent_tx,
                revision_tx,
            },
            intent_rx,
        )
    }

    pub(crate) fn snapshot(&self) -> RuntimeSnapshot {
        let inner = self.inner.lock().expect("service runtime mutex poisoned");
        RuntimeSnapshot {
            revision: inner.revision,
            state: inner.state,
            operations: inner.operations.iter().cloned().collect(),
            last_error: inner.last_error.clone(),
        }
    }

    pub(crate) fn subscribe(&self) -> watch::Receiver<u64> {
        self.revision_tx.subscribe()
    }

    pub(crate) fn publish_policy(&self, policy: &SessionPolicy) {
        let mut inner = self.inner.lock().expect("service runtime mutex poisoned");
        let mut state = policy.snapshot();
        // Output health belongs to the service-scoped writer. A short-lived
        // control connection must not overwrite a detected writer crash with
        // its freshly constructed default policy.
        if inner.state.output == OutputState::Failed
            || state.output == OutputState::Missing && inner.state.output != OutputState::Missing
        {
            state.output = inner.state.output;
        }
        if inner.state == state {
            return;
        }
        inner.state = state;
        Self::advance(&mut inner, &self.revision_tx);
    }

    pub(crate) fn set_output_ready(&self) {
        self.set_output_state(OutputState::ReadyNeutral);
    }

    pub(crate) fn set_output_failed(&self, message: impl Into<String>) {
        let mut inner = self.inner.lock().expect("service runtime mutex poisoned");
        inner.state.output = OutputState::Failed;
        inner.last_error = Some(OperationSnapshot {
            id: "output".to_owned(),
            intent: "output_supervision",
            state: OperationState::Failed,
            error_code: Some("output_unavailable"),
            message: Some(message.into()),
        });
        Self::advance(&mut inner, &self.revision_tx);
    }

    fn set_output_state(&self, output: OutputState) {
        let mut inner = self.inner.lock().expect("service runtime mutex poisoned");
        if inner.state.output == output {
            return;
        }
        inner.state.output = output;
        Self::advance(&mut inner, &self.revision_tx);
    }

    /// Invalidates authorization before transport and resource cleanup begins.
    pub(crate) fn invalidate_stop(&self) {
        let mut inner = self.inner.lock().expect("service runtime mutex poisoned");
        let active = inner.state.capture != CaptureState::Idle || inner.state.capture_armed;
        let mut cancelled_start = None;
        for operation in &mut inner.operations {
            if operation.intent == "request_start" && operation.state == OperationState::Pending {
                operation.state = OperationState::Failed;
                operation.error_code = Some("cancelled");
                operation.message = Some("Start was cancelled by Stop".to_owned());
                cancelled_start = Some(operation.clone());
            }
        }
        if let Some(operation) = &cancelled_start {
            inner.last_error = Some(operation.clone());
        }
        if active {
            inner.state.generation = inner.state.generation.wrapping_add(1);
        }
        inner.state.capture = CaptureState::Idle;
        inner.state.capture_armed = false;
        if inner.state.output == OutputState::ReadyLive {
            inner.state.output = OutputState::ReadyNeutral;
        }
        if active || cancelled_start.is_some() {
            Self::advance(&mut inner, &self.revision_tx);
        }
    }

    pub(crate) fn invalidate_forget(&self) {
        self.invalidate_stop();
        let mut inner = self.inner.lock().expect("service runtime mutex poisoned");
        inner.state.trust = TrustState::Revoked;
        inner.state.connection = ConnectionState::Offline;
        Self::advance(&mut inner, &self.revision_tx);
    }

    pub(crate) fn connection_lost(&self) {
        self.invalidate_stop();
        let mut inner = self.inner.lock().expect("service runtime mutex poisoned");
        inner.state.connection = if inner.state.trust == TrustState::Revoked {
            ConnectionState::Offline
        } else {
            ConnectionState::Recovering
        };
        Self::advance(&mut inner, &self.revision_tx);
    }

    pub(crate) async fn request_start(&self, operation_id: String) -> Result<(), &'static str> {
        if self.is_existing(&operation_id, "request_start")? {
            return Ok(());
        }
        let state = self.snapshot().state;
        if state.trust != TrustState::Trusted {
            return Err("peer_not_trusted");
        }
        if state.connection != omacam_core::ConnectionState::Online {
            return Err("peer_offline");
        }
        if state.capture != CaptureState::Idle {
            return Err("capture_busy");
        }
        if matches!(state.output, OutputState::Missing | OutputState::Failed) {
            return Err("output_unavailable");
        }
        if !self.enqueue(operation_id.clone(), "request_start")? {
            return Ok(());
        }
        if self
            .intent_tx
            .send(ServiceIntent::RequestStart {
                operation_id: operation_id.clone(),
            })
            .await
            .is_err()
        {
            self.fail(
                &operation_id,
                "service_unavailable",
                "service intent receiver stopped",
            );
            return Err("service_unavailable");
        }
        Ok(())
    }

    pub(crate) async fn stop(&self, operation_id: String) -> Result<(), &'static str> {
        if !self.enqueue(operation_id.clone(), "stop")? {
            return Ok(());
        }
        // Authorization is invalidated synchronously with accepting Stop, before
        // any transport notification or resource cleanup can run.
        self.invalidate_stop();
        if self
            .intent_tx
            .send(ServiceIntent::Stop {
                operation_id: operation_id.clone(),
            })
            .await
            .is_err()
        {
            self.fail(
                &operation_id,
                "service_unavailable",
                "service intent receiver stopped",
            );
            return Err("service_unavailable");
        }
        Ok(())
    }

    pub(crate) async fn forget_peer(&self, operation_id: String) -> Result<(), &'static str> {
        if !self.enqueue(operation_id.clone(), "forget_peer")? {
            return Ok(());
        }
        // Revoke locally before persistence removal or remote cleanup.
        self.invalidate_forget();
        if self
            .intent_tx
            .send(ServiceIntent::ForgetPeer {
                operation_id: operation_id.clone(),
            })
            .await
            .is_err()
        {
            self.fail(
                &operation_id,
                "service_unavailable",
                "service intent receiver stopped",
            );
            return Err("service_unavailable");
        }
        Ok(())
    }

    pub(crate) fn run_diagnostics(&self, operation_id: &str) -> Result<(), &'static str> {
        if !self.enqueue(operation_id.to_owned(), "run_diagnostics")? {
            return Ok(());
        }
        self.succeed(operation_id);
        Ok(())
    }

    fn enqueue(&self, operation_id: String, intent: &'static str) -> Result<bool, &'static str> {
        let mut inner = self.inner.lock().expect("service runtime mutex poisoned");
        if let Some(existing) = inner.operations.iter().find(|item| item.id == operation_id) {
            return if existing.intent == intent {
                Ok(false)
            } else {
                Err("operation_id_conflict")
            };
        }
        if operation_id.is_empty() || operation_id.len() > 64 {
            return Err("invalid_operation_id");
        }
        inner.operations.push_back(OperationSnapshot {
            id: operation_id,
            intent,
            state: OperationState::Pending,
            error_code: None,
            message: None,
        });
        while inner.operations.len() > MAX_OPERATIONS {
            inner.operations.pop_front();
        }
        Self::advance(&mut inner, &self.revision_tx);
        Ok(true)
    }

    fn is_existing(&self, operation_id: &str, intent: &'static str) -> Result<bool, &'static str> {
        let inner = self.inner.lock().expect("service runtime mutex poisoned");
        match inner.operations.iter().find(|item| item.id == operation_id) {
            Some(existing) if existing.intent == intent => Ok(true),
            Some(_) => Err("operation_id_conflict"),
            None => Ok(false),
        }
    }

    pub(crate) fn succeed(&self, operation_id: &str) {
        self.finish(operation_id, OperationState::Succeeded, None, None);
    }

    pub(crate) fn fail(&self, operation_id: &str, code: &'static str, message: impl Into<String>) {
        self.finish(
            operation_id,
            OperationState::Failed,
            Some(code),
            Some(message.into()),
        );
    }

    fn finish(
        &self,
        operation_id: &str,
        state: OperationState,
        error_code: Option<&'static str>,
        message: Option<String>,
    ) {
        let mut inner = self.inner.lock().expect("service runtime mutex poisoned");
        let Some(operation) = inner
            .operations
            .iter_mut()
            .find(|item| item.id == operation_id)
        else {
            return;
        };
        if operation.state != OperationState::Pending {
            return;
        }
        operation.state = state;
        operation.error_code = error_code;
        operation.message = message;
        if operation.state == OperationState::Failed {
            inner.last_error = Some(operation.clone());
        }
        Self::advance(&mut inner, &self.revision_tx);
    }

    fn advance(inner: &mut RuntimeInner, revision_tx: &watch::Sender<u64>) {
        inner.revision = inner.revision.saturating_add(1);
        revision_tx.send_replace(inner.revision);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn intents_are_bounded_idempotent_and_do_not_claim_success() {
        let mut policy = SessionPolicy::default();
        policy.set_output_ready();
        policy.trust_peer();
        policy.authenticate_connection().expect("online");
        let (runtime, mut rx) = ServiceRuntime::new(&policy, "trust.json".into());

        runtime
            .request_start("op-1".into())
            .await
            .expect("accepted");
        policy.request_start().expect("pending consent");
        runtime.publish_policy(&policy);
        runtime
            .request_start("op-1".into())
            .await
            .expect("idempotent");
        assert_eq!(runtime.snapshot().operations.len(), 1);
        assert_eq!(
            runtime.snapshot().operations[0].state,
            OperationState::Pending
        );
        assert!(matches!(
            rx.recv().await,
            Some(ServiceIntent::RequestStart { .. })
        ));
        runtime.succeed("op-1");
        assert_eq!(
            runtime.snapshot().operations[0].state,
            OperationState::Succeeded
        );
    }

    #[tokio::test]
    async fn start_rejects_false_preconditions_with_typed_codes() {
        let (runtime, _) = ServiceRuntime::new(&SessionPolicy::default(), "trust.json".into());
        assert_eq!(
            runtime.request_start("op".into()).await,
            Err("peer_not_trusted")
        );
    }

    #[tokio::test]
    async fn stop_cancels_pending_start_before_the_cleanup_intent_is_observed() {
        let mut policy = SessionPolicy::default();
        policy.set_output_ready();
        policy.trust_peer();
        policy.authenticate_connection().expect("online");
        let (runtime, mut rx) = ServiceRuntime::new(&policy, "trust.json".into());
        runtime.request_start("start".into()).await.expect("start");
        assert!(matches!(
            rx.recv().await,
            Some(ServiceIntent::RequestStart { .. })
        ));

        runtime.stop("stop".into()).await.expect("stop");
        let snapshot = runtime.snapshot();
        let start = snapshot
            .operations
            .iter()
            .find(|item| item.id == "start")
            .unwrap();
        assert_eq!(start.state, OperationState::Failed);
        assert_eq!(start.error_code, Some("cancelled"));
        assert!(!snapshot.state.capture_armed);
        assert!(matches!(rx.recv().await, Some(ServiceIntent::Stop { .. })));
    }

    #[tokio::test]
    async fn closed_intent_receiver_finishes_operations_with_a_typed_failure() {
        let mut policy = SessionPolicy::default();
        policy.trust_peer();
        policy.authenticate_connection().expect("online");
        let (runtime, rx) = ServiceRuntime::new(&policy, "trust.json".into());
        drop(rx);

        assert_eq!(
            runtime.stop("stop".into()).await,
            Err("service_unavailable")
        );
        let operation = runtime.snapshot().operations.into_iter().next().unwrap();
        assert_eq!(operation.state, OperationState::Failed);
        assert_eq!(operation.error_code, Some("service_unavailable"));
    }

    #[test]
    fn operation_history_is_bounded_and_ids_cannot_change_intent() {
        let (runtime, _) = ServiceRuntime::new(&SessionPolicy::default(), "trust.json".into());
        for index in 0..(MAX_OPERATIONS + 5) {
            runtime
                .run_diagnostics(&format!("diagnostic-{index}"))
                .expect("diagnostic operation");
        }
        let snapshot = runtime.snapshot();
        assert_eq!(snapshot.operations.len(), MAX_OPERATIONS);
        assert_eq!(snapshot.operations[0].id, "diagnostic-5");
        assert_eq!(
            runtime.enqueue("diagnostic-5".to_owned(), "stop"),
            Err("operation_id_conflict")
        );
        assert_eq!(
            runtime.enqueue("x".repeat(65), "stop"),
            Err("invalid_operation_id")
        );
    }

    #[test]
    fn stop_and_forget_invalidate_and_advance_revision() {
        let mut policy = SessionPolicy::default();
        policy.trust_peer();
        policy.authenticate_connection().expect("online");
        policy.request_start().expect("request");
        policy.grant_consent(0).expect("consent");
        let (runtime, _) = ServiceRuntime::new(&policy, "trust.json".into());
        let initial = runtime.snapshot();
        runtime.invalidate_stop();
        let stopped = runtime.snapshot();
        assert!(stopped.revision > initial.revision);
        assert_eq!(stopped.state.capture, CaptureState::Idle);
        assert!(!stopped.state.capture_armed);
        assert!(stopped.state.generation > initial.state.generation);
        runtime.invalidate_forget();
        let forgotten = runtime.snapshot();
        assert_eq!(forgotten.state.trust, TrustState::Revoked);
        assert_eq!(forgotten.state.connection, ConnectionState::Offline);
    }

    #[test]
    fn output_failure_is_not_overwritten_by_a_new_connection_policy() {
        let mut policy = SessionPolicy::default();
        policy.set_output_ready();
        policy.trust_peer();
        policy.authenticate_connection().expect("online");
        let (runtime, _) = ServiceRuntime::new(&policy, "trust.json".into());
        runtime.set_output_failed("writer exited");

        let mut fresh_policy = SessionPolicy::default();
        fresh_policy.trust_peer();
        fresh_policy.authenticate_connection().expect("online");
        runtime.publish_policy(&fresh_policy);

        assert_eq!(runtime.snapshot().state.output, OutputState::Failed);
        assert_eq!(
            runtime.snapshot().last_error.as_ref().unwrap().error_code,
            Some("output_unavailable")
        );
    }

    #[tokio::test]
    async fn stop_then_start_intents_remain_observable_on_the_same_service_queue() {
        let mut policy = SessionPolicy::default();
        policy.set_output_ready();
        policy.trust_peer();
        policy.authenticate_connection().expect("online");
        let (runtime, mut receiver) = ServiceRuntime::new(&policy, "trust.json".into());

        runtime.stop("stop-1".into()).await.expect("stop accepted");
        runtime
            .request_start("start-1".into())
            .await
            .expect("start accepted after stop");

        assert!(matches!(
            receiver.recv().await,
            Some(ServiceIntent::Stop { operation_id }) if operation_id == "stop-1"
        ));
        assert!(matches!(
            receiver.recv().await,
            Some(ServiceIntent::RequestStart { operation_id }) if operation_id == "start-1"
        ));
    }

    #[tokio::test]
    async fn start_rejects_when_the_service_output_is_not_healthy() {
        let mut policy = SessionPolicy::default();
        policy.trust_peer();
        policy.authenticate_connection().expect("online");
        let (runtime, _) = ServiceRuntime::new(&policy, "trust.json".into());

        assert_eq!(
            runtime.request_start("start-1".into()).await,
            Err("output_unavailable")
        );
        assert!(runtime.snapshot().operations.is_empty());

        runtime.set_output_failed("writer exited");
        assert_eq!(
            runtime.request_start("start-2".into()).await,
            Err("output_unavailable")
        );
        assert!(runtime.snapshot().operations.is_empty());
    }
}

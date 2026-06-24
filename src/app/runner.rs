use std::path::PathBuf;
use std::time::Duration;

use tracing::{error, info, warn};

use super::mode::RunMode;
use super::shutdown::shutdown_signal;
use crate::cli::default_config_path;
use crate::config::Config;
use crate::gateway::{GatewayClient, SubAccountRegistry, TaskView, TemplateRegistry};
use crate::hyperliquid::HlExchangeClient;
use crate::observe::debug_http;
use crate::observe::debug_http::{DebugSnapshot, DebugStatus};
use crate::signing::NodeSigner;
use crate::state::{now_secs, StateStore};
use crate::{NodeError, Result};

const TEMPLATE_REFRESH_INTERVAL_SECS: i64 = 5 * 60;

/// Starts the long-running node daemon.
///
/// # Errors
///
/// Returns an error when configuration loading, signer initialization, gateway
/// authentication, local state setup, or a polling cycle fails.
pub async fn run(config_path: PathBuf) -> Result<()> {
    let mut runner = Runner::start(config_path, None).await?;
    let rpc_gateway = rpc_gateway_client(&runner.config, &runner.signer).await?;
    let _debug = debug_http::spawn(
        runner.config.rpc_http_addr,
        runner.debug.clone(),
        runner.state.clone(),
        runner.config.redacted(),
        runner.config.clone(),
        runner.signer.clone(),
        runner.templates.clone(),
        runner.sub_accounts.clone(),
        rpc_gateway,
    );
    runner.run_loop().await
}

/// Runs a single polling cycle.
///
/// # Errors
///
/// Returns an error when startup fails or the pending/executable processing
/// cycle cannot be completed.
pub async fn run_once(config_path: PathBuf, dry_run: bool) -> Result<()> {
    let mut runner = Runner::start(config_path, Some(dry_run)).await?;
    runner.process_cycle_with_auth_recovery().await
}

pub(super) struct Runner {
    pub(super) config: Config,
    pub(super) signer: NodeSigner,
    pub(super) gateway: GatewayClient,
    pub(super) hl: HlExchangeClient,
    pub(super) state: StateStore,
    pub(super) mode: RunMode,
    pub(super) debug: DebugSnapshot,
    pub(super) templates: TemplateRegistry,
    pub(super) sub_accounts: SubAccountRegistry,
    pub(super) consecutive_gateway_failures: u64,
    pub(super) last_template_refresh_at: Option<i64>,
}

impl Runner {
    async fn start(config_path: PathBuf, dry_run_override: Option<bool>) -> Result<Self> {
        let config_path = if config_path.as_os_str().is_empty() {
            default_config_path()
        } else {
            config_path
        };
        let mut config = Config::load(&config_path)?;
        if let Some(dry_run) = dry_run_override {
            config.dry_run = dry_run;
        }

        let signer = NodeSigner::decrypt(
            std::path::Path::new(&config.signer.keystore_path),
            config.signer.password_env.as_deref(),
        )?;
        let mode = mode_for_signer(signer.address_lc(), &config.allowed_leaders);
        let state = StateStore::connect(&config.state_db).await?;
        let mut gateway = GatewayClient::new(config.gateway_url.clone());
        let templates = TemplateRegistry::new(gateway.templates().await?);
        templates.validate_allowed_templates(&config.allowed_templates)?;
        templates.validate_template_input_policies(&config.template_input_policies)?;
        authenticate_gateway(&mut gateway, &signer, &config).await?;
        let sub_accounts = fetch_sub_accounts_if_needed(&mut gateway, &config).await?;
        log_send_asset_sub_account_snapshot(&config, &sub_accounts);

        let debug = DebugSnapshot::new(DebugStatus {
            signer: signer.address_lc().to_string(),
            mode: mode.as_str().to_string(),
            multisig: config.multisig.clone(),
            leader: config.leader.clone(),
            last_poll_at: None,
            last_success_at: None,
            last_error: None,
            last_error_at: None,
            consecutive_gateway_failures: 0,
        });

        info!(
            signer = signer.address_lc(),
            multisig = config.multisig,
            leader = config.leader,
            mode = mode.as_str(),
            dry_run = config.dry_run,
            "safe-node started"
        );

        Ok(Self {
            hl: HlExchangeClient::new(config.hl_api_url.clone()),
            config,
            signer,
            gateway,
            state,
            mode,
            debug,
            templates,
            sub_accounts,
            consecutive_gateway_failures: 0,
            last_template_refresh_at: Some(now_secs()),
        })
    }

    async fn run_loop(&mut self) -> Result<()> {
        let mut interval =
            tokio::time::interval(Duration::from_secs(self.config.poll_interval_secs.max(1)));
        loop {
            tokio::select! {
                () = shutdown_signal() => {
                    info!("shutdown signal received; stopping new polling cycles");
                    return Ok(());
                }
                _ = interval.tick() => {
                    if let Err(err) = self.process_cycle_with_auth_recovery().await {
                        self.record_cycle_error(&err).await;
                    }
                }
            }
        }
    }

    async fn process_cycle_with_auth_recovery(&mut self) -> Result<()> {
        let err = match self.process_cycle().await {
            Ok(()) => return Ok(()),
            Err(err) => err,
        };

        if gateway_session_renewal_required(&err) {
            self.relogin_after_session_ceiling(&err).await?;
        } else if gateway_authentication_failed(&err) {
            self.relogin_after_authentication_failure(&err).await?;
        } else {
            return Err(err);
        }

        self.process_cycle().await
    }

    pub(super) async fn process_cycle(&mut self) -> Result<()> {
        self.debug.mark_poll().await;
        self.refresh_templates_if_due().await;
        self.refresh_sub_accounts().await?;
        self.process_pending().await?;
        self.process_executable().await?;
        self.consecutive_gateway_failures = 0;
        self.debug.mark_success().await;
        Ok(())
    }

    async fn refresh_templates_if_due(&mut self) {
        let now = now_secs();
        if self
            .last_template_refresh_at
            .map(|last| now.saturating_sub(last) < TEMPLATE_REFRESH_INTERVAL_SECS)
            .unwrap_or(false)
        {
            return;
        }

        match self.gateway.templates().await {
            Ok(templates) => {
                let refreshed = TemplateRegistry::new(templates);
                match refreshed.validate_allowed_templates(&self.config.allowed_templates) {
                    Ok(()) => match refreshed
                        .validate_template_input_policies(&self.config.template_input_policies)
                    {
                        Ok(()) => {
                            self.templates = refreshed;
                            self.last_template_refresh_at = Some(now);
                        }
                        Err(err) => {
                            self.last_template_refresh_at = Some(now);
                            warn!(
                                error = %err,
                                "template refresh conflicted with configured input policies; \
                                 keeping previous registry"
                            );
                        }
                    },
                    Err(err) => {
                        self.last_template_refresh_at = Some(now);
                        warn!(
                            error = %err,
                            "template refresh returned invalid metadata; keeping previous registry"
                        );
                    }
                }
            }
            Err(err) => {
                self.last_template_refresh_at = Some(now);
                warn!(
                    error = %err,
                    "template refresh failed; keeping previous registry"
                );
            }
        }
    }

    async fn refresh_sub_accounts(&mut self) -> Result<()> {
        self.sub_accounts = fetch_sub_accounts_if_needed(&mut self.gateway, &self.config).await?;
        Ok(())
    }

    async fn record_cycle_error(&mut self, err: &NodeError) {
        if err.retryable() {
            self.consecutive_gateway_failures = self.consecutive_gateway_failures.saturating_add(1);
            if self.consecutive_gateway_failures == 1 {
                warn!(
                    error = %err,
                    consecutive_gateway_failures = self.consecutive_gateway_failures,
                    "polling cycle failed with retryable gateway error"
                );
            } else {
                error!(
                    error = %err,
                    consecutive_gateway_failures = self.consecutive_gateway_failures,
                    "polling cycle failed with repeated retryable gateway error"
                );
            }
        } else {
            self.consecutive_gateway_failures = 0;
            error!(error = %err, "polling cycle failed");
        }
        self.debug
            .mark_error(err.to_string(), self.consecutive_gateway_failures)
            .await;
    }

    async fn relogin_after_session_ceiling(&mut self, err: &NodeError) -> Result<()> {
        self.consecutive_gateway_failures = 0;
        info!(
            error = %err,
            "gateway session refresh reached lifetime ceiling; re-login"
        );
        self.reauthenticate_gateway().await.map_err(|reauth_err| {
            error!(
                error = %err,
                reauth_error = %reauth_err,
                "gateway session renewal failed"
            );
            NodeError::Runtime(format!("{err}; re-login failed: {reauth_err}"))
        })?;
        info!("gateway session restored; retrying polling cycle");
        Ok(())
    }

    async fn relogin_after_authentication_failure(&mut self, err: &NodeError) -> Result<()> {
        self.consecutive_gateway_failures = 0;
        warn!(
            error = %err,
            "gateway authentication failed; attempting re-login"
        );
        self.reauthenticate_gateway().await.map_err(|reauth_err| {
            error!(
                error = %err,
                reauth_error = %reauth_err,
                "gateway authentication re-login failed"
            );
            NodeError::Runtime(format!("{err}; re-login failed: {reauth_err}"))
        })?;
        info!("gateway authentication restored; retrying polling cycle");
        Ok(())
    }

    async fn reauthenticate_gateway(&mut self) -> Result<()> {
        authenticate_gateway(&mut self.gateway, &self.signer, &self.config).await
    }

    pub(super) async fn submit_policy_reject(
        &mut self,
        task: &TaskView,
        reason: Option<&str>,
    ) -> Result<()> {
        let reason = reason.unwrap_or("policy reject");
        if self.config.dry_run {
            self.state.record_rejected(task, reason).await?;
            info!(
                task_id = task.id,
                reject_reason = reason,
                "dry-run policy reject; not submitting gateway reject vote"
            );
            return Ok(());
        }

        let signer = self.signer.address_lc().to_string();
        match self.gateway.reject_task(&task.id, &signer, reason).await {
            Ok(updated) => {
                self.state.record_rejected(&updated, reason).await?;
                info!(
                    task_id = updated.id,
                    signer = signer,
                    reject_reason = reason,
                    gateway_status = updated.status,
                    gateway_rejects = updated.rejects,
                    gateway_rejections = updated.rejections.len(),
                    "confirmed gateway reject vote"
                );
            }
            Err(err) if gateway_reject_is_unavailable(&err) => {
                let local_reason = format!("gateway reject unavailable: {err}");
                self.state.record_ignored(task, &local_reason).await?;
                warn!(
                    task_id = task.id,
                    reject_reason = reason,
                    error = %err,
                    "gateway reject vote was unavailable; task ignored locally"
                );
            }
            Err(err) => {
                self.state.record_failed(task, &err.to_string()).await?;
                return Err(err);
            }
        }
        Ok(())
    }
}

fn gateway_reject_is_unavailable(err: &NodeError) -> bool {
    matches!(err, NodeError::GatewayBusiness { code: 4206, .. })
}

fn gateway_authentication_failed(err: &NodeError) -> bool {
    matches!(err, NodeError::Unauthorized)
}

fn gateway_session_renewal_required(err: &NodeError) -> bool {
    matches!(err, NodeError::SessionRenewalRequired)
}

pub(super) fn mode_for_signer(signer_lc: &str, allowed_leaders: &[String]) -> RunMode {
    if allowed_leaders.iter().any(|leader| leader == signer_lc) {
        RunMode::LeaderExecutor
    } else {
        RunMode::CoSigner
    }
}

async fn authenticate_gateway(
    gateway: &mut GatewayClient,
    signer: &NodeSigner,
    config: &Config,
) -> Result<()> {
    gateway.login(signer).await?;
    let account = gateway.track_account(&config.multisig).await?;
    if !account.has_signer(signer.address_lc()) {
        return Err(NodeError::Runtime(format!(
            "signer {} is not an authorized signer of multisig {}",
            signer.address_lc(),
            config.multisig
        )));
    }
    Ok(())
}

async fn rpc_gateway_client(config: &Config, signer: &NodeSigner) -> Result<GatewayClient> {
    let mut gateway = GatewayClient::new(config.gateway_url.clone());
    authenticate_gateway(&mut gateway, signer, config).await?;
    Ok(gateway)
}

async fn fetch_sub_accounts_if_needed(
    gateway: &mut GatewayClient,
    config: &Config,
) -> Result<SubAccountRegistry> {
    if !send_asset_enabled(config) {
        return Ok(SubAccountRegistry::default());
    }
    gateway.sub_accounts(&config.multisig).await
}

fn send_asset_enabled(config: &Config) -> bool {
    config
        .allowed_templates
        .iter()
        .any(|template| template == "send_asset")
}

fn log_send_asset_sub_account_snapshot(config: &Config, sub_accounts: &SubAccountRegistry) {
    if !send_asset_enabled(config) {
        return;
    }
    if sub_accounts.is_empty() {
        warn!(
            multisig = config.multisig,
            "send_asset is enabled but no sub-accounts are cached; send_asset tasks will be rejected"
        );
    } else {
        info!(
            multisig = config.multisig,
            sub_account_count = sub_accounts.len(),
            "loaded send_asset sub-account allowlist"
        );
    }
}

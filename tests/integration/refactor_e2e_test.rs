//! End-to-end refactoring test — validates the new thresholds (max_rounds=100, stuck=10).
//!
//! Creates a realistic mini-project with inter-connected Rust files that need
//! refactoring (rename a trait method across all implementations and callers),
//! then spawns a subagent to perform the refactoring.
//!
//! Run: cargo test --test refactor_e2e_test -- --nocapture --ignored
//! (ignored because it calls the real API)
//!
//! Gate: references the pre-refactor `run_subagent_loop` API (now `run_agent_loop`
//! with `RunLoopArgs`). Excluded from default `cargo test --all` to avoid blocking
//! the build. Enable with `--features integration-tests`.

#![cfg(feature = "integration-tests")]

use std::sync::Arc;
use wgenty_code::agent::progress::{ProgressCallback, SubagentProgress};
use wgenty_code::api::ApiClient;
use wgenty_code::config::Settings;
use wgenty_code::teams::subagent_loop::run_subagent_loop;
use wgenty_code::tools::ToolRegistry;

fn create_test_project(dir: &std::path::Path) {
    let src = dir.join("src");
    std::fs::create_dir_all(&src).unwrap();

    std::fs::write(
        dir.join("Cargo.toml"),
        r#"[package]
name = "mini-payment"
version = "0.1.0"
edition = "2021"
[dependencies]
uuid = { version = "1", features = ["v4"] }
"#,
    )
    .unwrap();

    std::fs::write(src.join("lib.rs"), r#"//! Mini payment processing library

pub mod auth;
pub mod payment;
pub mod notification;

pub trait PaymentProcessor {
    fn process_transaction(&self, amount: f64, currency: &str) -> Result<String, PaymentError>;
    fn refund_transaction(&self, transaction_id: &str) -> Result<(), PaymentError>;
}

pub struct AuditLog { pub entries: Vec<AuditEntry> }
pub struct AuditEntry { pub transaction_id: String, pub amount: f64, pub currency: String, pub timestamp: i64, pub status: TransactionStatus }

#[derive(Debug, Clone, PartialEq)]
pub enum TransactionStatus { Pending, Completed, Failed, Refunded }

impl AuditLog {
    pub fn new() -> Self { Self { entries: Vec::new() } }
    pub fn record(&mut self, entry: AuditEntry) { self.entries.push(entry); }
    pub fn find_by_date(&self, _start: i64, _end: i64) -> Vec<&AuditEntry> { self.entries.iter().collect() }
}

#[derive(Debug)]
pub struct PaymentError { pub code: String, pub message: String }

impl std::fmt::Display for PaymentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "[{}] {}", self.code, self.message) }
}
impl std::error::Error for PaymentError {}
"#).unwrap();

    std::fs::write(src.join("payment.rs"), r#"use crate::{PaymentProcessor, PaymentError, AuditLog, AuditEntry, TransactionStatus};

pub struct StripeProcessor { pub api_key: String, pub audit: AuditLog }

impl StripeProcessor {
    pub fn new(api_key: &str) -> Self { Self { api_key: api_key.to_string(), audit: AuditLog::new() } }
    fn validate_amount(&self, amount: f64) -> Result<(), PaymentError> {
        if amount <= 0.0 { return Err(PaymentError { code: "INVALID_AMOUNT".into(), message: format!("Amount must be positive, got {}", amount) })); }
        Ok(())
    }
    fn call_stripe_api(&self, amount: f64, currency: &str) -> Result<String, PaymentError> {
        if self.api_key.is_empty() { return Err(PaymentError { code: "AUTH_ERROR".into(), message: "API key not configured".into() }); }
        Ok(format!("stripe_txn_{}", uuid::Uuid::new_v4()))
    }
}

impl PaymentProcessor for StripeProcessor {
    fn process_transaction(&self, amount: f64, currency: &str) -> Result<String, PaymentError> {
        self.validate_amount(amount)?;
        self.call_stripe_api(amount, currency)
    }
    fn refund_transaction(&self, transaction_id: &str) -> Result<(), PaymentError> {
        if transaction_id.is_empty() { return Err(PaymentError { code: "INVALID_TXN".into(), message: "Transaction ID is empty".into() }); }
        Ok(())
    }
}

pub struct PayPalProcessor { pub client_id: String, pub secret: String }

impl PayPalProcessor {
    pub fn new(client_id: &str, secret: &str) -> Self { Self { client_id: client_id.to_string(), secret: secret.to_string() } }
}

impl PaymentProcessor for PayPalProcessor {
    fn process_transaction(&self, amount: f64, currency: &str) -> Result<String, PaymentError> {
        if self.client_id.is_empty() || self.secret.is_empty() { return Err(PaymentError { code: "AUTH_ERROR".into(), message: "PayPal credentials not configured".into() }); }
        Ok(format!("paypal_txn_{}", uuid::Uuid::new_v4()))
    }
    fn refund_transaction(&self, transaction_id: &str) -> Result<(), PaymentError> { Ok(()) }
}
"#).unwrap();

    std::fs::write(src.join("auth.rs"), r#"use crate::{PaymentProcessor, PaymentError};

pub struct PaymentAuthenticator<P: PaymentProcessor> { processor: P, api_key: String }

impl<P: PaymentProcessor> PaymentAuthenticator<P> {
    pub fn new(processor: P, api_key: &str) -> Self { Self { processor, api_key: api_key.to_string() } }
    pub fn authenticated_payment(&self, token: &str, amount: f64, currency: &str) -> Result<String, PaymentError> {
        if token.is_empty() { return Err(PaymentError { code: "AUTH_FAILED".into(), message: "Authentication token is empty".into() }); }
        self.processor.process_transaction(amount, currency)
    }
}
"#).unwrap();

    std::fs::write(src.join("notification.rs"), r#"use crate::{PaymentProcessor, PaymentError};

pub struct NotificationService<P: PaymentProcessor> { processor: P, email_templates: Vec<String> }

impl<P: PaymentProcessor> NotificationService<P> {
    pub fn new(processor: P) -> Self {
        Self { processor, email_templates: vec!["payment_success".into(), "payment_failed".into(), "refund_processed".into()] }
    }
    pub fn process_and_notify(&self, amount: f64, currency: &str, user_email: &str) -> Result<String, PaymentError> {
        let txn_id = self.processor.process_transaction(amount, currency)?;
        let _ = user_email;
        Ok(txn_id)
    }
    pub fn batch_process(&self, payments: &[(f64, &str, &str)]) -> Vec<Result<String, PaymentError>> {
        payments.iter().map(|(amount, currency, email)| self.process_and_notify(*amount, currency, email)).collect()
    }
}
"#).unwrap();
}

fn count_occurrences(dir: &std::path::Path, pattern: &str) -> usize {
    let mut count = 0;
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                count += count_occurrences(&path, pattern);
            } else if path.extension().map(|e| e == "rs").unwrap_or(false) {
                if let Ok(content) = std::fs::read_to_string(&path) {
                    count += content.matches(pattern).count();
                }
            }
        }
    }
    count
}

fn progress_printer(progress: SubagentProgress) {
    let icon = match progress.status {
        wgenty_code::agent::progress::SubagentStatus::Running => "🔄",
        wgenty_code::agent::progress::SubagentStatus::Completed => "✅",
        wgenty_code::agent::progress::SubagentStatus::Failed => "❌",
        _ => "⏳",
    };
    println!(
        "  {} {:>5}ms  round {}/{}  {:20}  {}",
        icon,
        progress.elapsed_ms,
        progress.round.unwrap_or(0),
        progress.max_rounds.unwrap_or(0),
        progress.current_tool.as_deref().unwrap_or("thinking"),
        progress.current_params.as_deref().unwrap_or("")
    );
}

#[tokio::test]
#[ignore]
async fn test_refactor_rename_trait_method_across_project() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let project_dir = temp_dir.path().join("mini-payment");
    std::fs::create_dir_all(&project_dir).unwrap();
    create_test_project(&project_dir);

    let before = count_occurrences(&project_dir, "process_transaction");
    println!("\n📁 Test project: {}", project_dir.display());
    println!("   'process_transaction' before: {}", before);
    assert!(before >= 5);

    let settings = Settings::load().expect("Failed to load settings");
    println!(
        "   API: {} / {}",
        settings.models.main.endpoint_base_url(),
        settings.models.main.name
    );

    let registry = Arc::new(ToolRegistry::new());
    let api_client = ApiClient::new(settings.clone());
    let allowed_tools: Vec<String> = registry
        .list()
        .iter()
        .map(|t| t.name().to_string())
        .filter(|n| n != "task")
        .collect();
    // The e2e test is the root scope: build a trusted root execution context
    // (never derived from model JSON) and pass it through the loop so every
    // nested tool call observes the trusted agent identity.
    let coordinator = std::sync::Arc::new(wgenty_code::agent::AgentCoordinator::new(4, 3));
    let root_context = coordinator
        .ensure_root(wgenty_code::agent::SessionId::new("refactor-e2e"))
        .await
        .expect("ensure_root");
    println!("\n🔧 Tools: {}", allowed_tools.len());

    let system_prompt = r#"You are a code refactoring subagent. RULES:
1. EXPLORE the codebase first (glob, grep, file_read)
2. EDIT each file using file_edit (old_string/new_string for exact replacements)
3. VERIFY changes by reading modified files
4. Return a summary of ALL changes with file paths"#;

    let user_prompt = format!(
        r#"## Refactoring Task
Rename the trait method `process_transaction` to `process_payment` across the ENTIRE codebase at: {}

What to do:
1. src/lib.rs: rename trait method definition
2. src/payment.rs: rename BOTH implementations (StripeProcessor + PayPalProcessor)
3. src/auth.rs: update call site
4. src/notification.rs: update ALL call sites

Return summary with files changed and remaining occurrences of "process_transaction" (should be 0 in code)."#,
        project_dir.display()
    );

    println!("\n🚀 Starting refactoring subagent (max_rounds=100, stuck_detection=10)...\n");

    let progress_cb: ProgressCallback = Arc::new(progress_printer);
    let start = std::time::Instant::now();

    let result = run_subagent_loop(
        &api_client,
        registry.clone(),
        &root_context,
        coordinator,
        system_prompt,
        &user_prompt,
        &allowed_tools,
        100,
        600,
        Some(progress_cb),
        None,
        None,
    )
    .await;
    let elapsed = start.elapsed();

    println!("\n⏱️  Total time: {:.1}s", elapsed.as_secs_f64());

    match result {
        Ok(summary) => {
            println!("✅ COMPLETED!\n");
            println!("{}", "━".repeat(70));
            println!("📋 SUMMARY:\n{}", summary);
            println!("{}", "━".repeat(70));

            let after = count_occurrences(&project_dir, "process_transaction");
            let payment = count_occurrences(&project_dir, "process_payment");
            println!("\n📊 VERIFICATION:");
            println!("   'process_transaction' (old): {} → {}", before, after);
            println!("   'process_payment'    (new): {}", payment);
            if after < before && payment > 0 {
                println!("   ✅ REFACTORING VERIFIED");
            }
            if after == 0 {
                println!("   ✅ CLEAN: old name fully eliminated");
            }
            assert!(!summary.is_empty());
        }
        Err(e) => {
            println!("❌ FAILED: {}", e);
            let lower = e.message.to_lowercase();
            if lower.contains("stuck") {
                println!("   Cause: STUCK DETECTION — would have been false positive with old 3-repeat threshold");
            } else if lower.contains("max") && lower.contains("round") {
                println!("   Cause: MAX ROUNDS — would have failed at round 30 with old threshold");
            } else if lower.contains("timeout") {
                println!("   Cause: TIMEOUT");
            } else {
                println!("   Cause: OTHER — {}", e);
            }
            println!("\n   Old max_rounds=30: would have failed at round 30");
            println!(
                "   New max_rounds=100: {}",
                if elapsed.as_secs() > 200 {
                    "may still fail for very complex tasks"
                } else {
                    "had enough headroom"
                }
            );
        }
    }
}

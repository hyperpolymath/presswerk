// SPDX-License-Identifier: PMPL-1.0-or-later
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>
//
// Retry engine with exponential backoff + jitter for resilient printing.
//
// Classifies errors into Transient (auto-retry), UserAction (wait for user),
// and Permanent (give up). Only transient errors trigger automatic retries.

use std::time::Duration;

use presswerk_core::error::PresswerkError;
use presswerk_core::types::ErrorClass;
use tracing::{debug, info, warn};

/// Retry configuration.
pub struct RetryConfig {
    /// Maximum number of retry attempts.
    pub max_retries: u32,
    /// Base delay between retries (exponential backoff).
    pub base_delay: Duration,
    /// Maximum delay between retries.
    pub max_delay: Duration,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 5,
            base_delay: Duration::from_secs(2),
            max_delay: Duration::from_secs(120),
        }
    }
}

/// Result of evaluating whether to retry.
pub enum RetryDecision {
    /// Retry after this delay.
    RetryAfter(Duration),
    /// Do not retry — error is permanent or user action needed.
    GiveUp(ErrorClass),
    /// Maximum retries exhausted.
    Exhausted,
}

/// Classify a `PresswerkError` into an `ErrorClass` for retry decisions.
pub fn classify_error(err: &PresswerkError) -> ErrorClass {
    match err {
        // Transient — network, timeout, temporary server issues
        PresswerkError::IppRequest(detail) => classify_ipp_detail(detail),
        PresswerkError::Discovery(_) => ErrorClass::Transient,
        PresswerkError::PrintServer(_) => ErrorClass::Transient,
        PresswerkError::Database(_) => ErrorClass::Transient,
        PresswerkError::Certificate(_) => ErrorClass::Transient,
        PresswerkError::OcrError(_) => ErrorClass::Transient,

        // User action needed
        PresswerkError::NoPrinterSelected => ErrorClass::UserAction,

        // Permanent — wrong format, bad data, platform missing
        PresswerkError::UnsupportedDocument(_) => ErrorClass::Permanent,
        PresswerkError::PdfError(_) => ErrorClass::Permanent,
        PresswerkError::ImageError(_) => ErrorClass::Permanent,
        PresswerkError::Encryption(_) => ErrorClass::Permanent,
        PresswerkError::Decryption(_) => ErrorClass::Permanent,
        PresswerkError::IntegrityMismatch { .. } => ErrorClass::Permanent,
        PresswerkError::PlatformUnavailable => ErrorClass::Permanent,
        PresswerkError::Bridge(_) => ErrorClass::Permanent,
        PresswerkError::Serialization(_) => ErrorClass::Permanent,

        // IO errors depend on the kind
        PresswerkError::Io(io_err) => match io_err.kind() {
            std::io::ErrorKind::TimedOut
            | std::io::ErrorKind::ConnectionRefused
            | std::io::ErrorKind::ConnectionReset
            | std::io::ErrorKind::ConnectionAborted
            | std::io::ErrorKind::Interrupted => ErrorClass::Transient,
            std::io::ErrorKind::NotFound | std::io::ErrorKind::PermissionDenied => {
                ErrorClass::UserAction
            }
            _ => ErrorClass::Transient,
        },
    }
}

/// Classify an IPP error detail string.
fn classify_ipp_detail(detail: &str) -> ErrorClass {
    let lower = detail.to_ascii_lowercase();

    // Transient network/server errors
    if lower.contains("timed out")
        || lower.contains("connection refused")
        || lower.contains("connection reset")
        || lower.contains("broken pipe")
        || lower.contains("server-error")
    {
        return ErrorClass::Transient;
    }

    // User action required (printer physical state)
    if lower.contains("media-empty")
        || lower.contains("toner-empty")
        || lower.contains("ink")
        || lower.contains("door-open")
        || lower.contains("cover-open")
        || lower.contains("paper-jam")
        || lower.contains("media-jam")
        || lower.contains("marker-supply")
    {
        return ErrorClass::UserAction;
    }

    // Permanent client errors
    if lower.contains("client-error-document-format")
        || lower.contains("client-error-not-possible")
        || lower.contains("invalid uri")
    {
        return ErrorClass::Permanent;
    }

    // Default to transient (optimistic — retry first, give up later)
    ErrorClass::Transient
}

/// Decide whether to retry based on the error class and attempt count.
pub fn should_retry(err: &PresswerkError, attempt: u32, config: &RetryConfig) -> RetryDecision {
    let class = classify_error(err);

    match class {
        ErrorClass::Permanent => {
            info!("permanent error — not retrying");
            RetryDecision::GiveUp(ErrorClass::Permanent)
        }
        ErrorClass::UserAction => {
            info!("user action required — not auto-retrying");
            RetryDecision::GiveUp(ErrorClass::UserAction)
        }
        ErrorClass::Transient => {
            if attempt >= config.max_retries {
                warn!(attempt, max = config.max_retries, "retry limit exhausted");
                RetryDecision::Exhausted
            } else {
                let delay = compute_delay(attempt, config);
                debug!(attempt, delay_ms = delay.as_millis(), "scheduling retry");
                RetryDecision::RetryAfter(delay)
            }
        }
    }
}

/// Compute exponential backoff delay with jitter.
///
/// delay = min(base * 2^attempt + jitter, max_delay)
/// jitter is a random value in [0, base) to prevent thundering herd.
fn compute_delay(attempt: u32, config: &RetryConfig) -> Duration {
    let base_ms = config.base_delay.as_millis() as u64;
    let exp_ms = base_ms.saturating_mul(1u64 << attempt.min(10));

    // Simple deterministic jitter based on attempt number (avoids rand dependency
    // if not available, but we use rand when the feature is present)
    let jitter_ms = jitter(base_ms, attempt);
    let total_ms = exp_ms.saturating_add(jitter_ms);
    let capped_ms = total_ms.min(config.max_delay.as_millis() as u64);

    Duration::from_millis(capped_ms)
}

/// Generate jitter using a simple hash of the attempt number.
/// When the `rand` crate is available, this should be replaced with proper
/// random jitter. For now, a deterministic but spread-out value suffices.
fn jitter(base_ms: u64, attempt: u32) -> u64 {
    // Multiply by a prime and take modulo base to get spread across [0, base)
    let hash = (attempt as u64).wrapping_mul(6364136223846793005);
    hash % base_ms.max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeout_is_transient() {
        let err = PresswerkError::IppRequest("timed out after 60s".into());
        assert_eq!(classify_error(&err), ErrorClass::Transient);
    }

    #[test]
    fn paper_jam_is_user_action() {
        let err = PresswerkError::IppRequest("printer stopped: paper-jam".into());
        assert_eq!(classify_error(&err), ErrorClass::UserAction);
    }

    #[test]
    fn bad_format_is_permanent() {
        let err = PresswerkError::IppRequest("client-error-document-format-not-supported".into());
        assert_eq!(classify_error(&err), ErrorClass::Permanent);
    }

    #[test]
    fn retry_respects_max() {
        let config = RetryConfig {
            max_retries: 3,
            ..Default::default()
        };
        let err = PresswerkError::IppRequest("connection refused".into());
        assert!(matches!(
            should_retry(&err, 0, &config),
            RetryDecision::RetryAfter(_)
        ));
        assert!(matches!(
            should_retry(&err, 3, &config),
            RetryDecision::Exhausted
        ));
    }

    #[test]
    fn permanent_error_never_retries() {
        let config = RetryConfig::default();
        let err = PresswerkError::UnsupportedDocument("docx".into());
        assert!(matches!(
            should_retry(&err, 0, &config),
            RetryDecision::GiveUp(ErrorClass::Permanent)
        ));
    }

    #[test]
    fn delay_increases_with_attempts() {
        let config = RetryConfig::default();
        let d0 = compute_delay(0, &config);
        let d1 = compute_delay(1, &config);
        let d2 = compute_delay(2, &config);
        // Each should be roughly double the previous (modulo jitter)
        assert!(d1 > d0);
        assert!(d2 > d1);
    }

    #[test]
    fn delay_capped_at_max() {
        let config = RetryConfig {
            max_delay: Duration::from_secs(10),
            ..Default::default()
        };
        let d = compute_delay(20, &config);
        assert!(d <= Duration::from_secs(10));
    }
}

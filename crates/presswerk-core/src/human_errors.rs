// SPDX-License-Identifier: PMPL-1.0-or-later
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>
//
// Human-readable error messages for vulnerable users (elderly, children).
//
// Every technical error is mapped to plain English with a clear suggestion.
// The taxonomy uses four severity levels that drive UI presentation.

use crate::error::PresswerkError;

/// Severity of an error from the user's perspective.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// Network blip, timeout — we can retry automatically.
    Transient,
    /// User must do something (add paper, close door, clear jam).
    ActionRequired,
    /// Cannot be fixed by retrying or user action — wrong format, bad URI, etc.
    Permanent,
    /// A physical purchase is needed (cable, ink, adapter).
    BuyRequired,
}

/// A human-readable error with plain English message and actionable suggestion.
#[derive(Debug, Clone)]
pub struct HumanError {
    /// Plain English summary (shown as a heading).
    pub message: String,
    /// What the user should try (shown as body text).
    pub suggestion: String,
    /// Whether the system should auto-retry.
    pub retriable: bool,
    /// Severity level (drives icon/colour in UI).
    pub severity: Severity,
}

/// Convert a `PresswerkError` into a `HumanError` that a grandparent can understand.
pub fn humanize_error(err: &PresswerkError) -> HumanError {
    match err {
        // -- Print errors --
        PresswerkError::Discovery(detail) => {
            if detail.contains("daemon") || detail.contains("multicast") {
                HumanError {
                    message: "We can't search for printers right now.".into(),
                    suggestion: "Make sure you're connected to Wi-Fi, then try again.".into(),
                    retriable: true,
                    severity: Severity::Transient,
                }
            } else {
                HumanError {
                    message: "We couldn't find any printers.".into(),
                    suggestion: "Make sure your printer is turned on and connected to the same Wi-Fi network as this device.".into(),
                    retriable: true,
                    severity: Severity::Transient,
                }
            }
        }

        PresswerkError::IppRequest(detail) => humanize_ipp_error(detail),

        PresswerkError::PrintServer(detail) => HumanError {
            message: "The print server had a problem.".into(),
            suggestion: format!("Try restarting the print server. ({detail})"),
            retriable: true,
            severity: Severity::Transient,
        },

        PresswerkError::NoPrinterSelected => HumanError {
            message: "No printer selected.".into(),
            suggestion: "Please choose a printer from the list, then try again.".into(),
            retriable: false,
            severity: Severity::ActionRequired,
        },

        // -- Document errors --
        PresswerkError::UnsupportedDocument(detail) => HumanError {
            message: "This type of document isn't supported.".into(),
            suggestion: format!("Try saving the file as a PDF first, then print the PDF. (File type: {detail})"),
            retriable: false,
            severity: Severity::Permanent,
        },

        PresswerkError::PdfError(_) => HumanError {
            message: "There's a problem with this PDF file.".into(),
            suggestion: "The file may be damaged. Try opening it on a computer first to check it works, or try a different file.".into(),
            retriable: false,
            severity: Severity::Permanent,
        },

        PresswerkError::ImageError(_) => HumanError {
            message: "There's a problem with this image.".into(),
            suggestion: "The image may be damaged or in an unusual format. Try saving it as a JPEG or PNG first.".into(),
            retriable: false,
            severity: Severity::Permanent,
        },

        PresswerkError::OcrError(_) => HumanError {
            message: "Text recognition didn't work on this scan.".into(),
            suggestion: "Try scanning the document again with better lighting, making sure the text is clear and in focus.".into(),
            retriable: true,
            severity: Severity::Transient,
        },

        // -- Security errors --
        PresswerkError::Encryption(_) | PresswerkError::Decryption(_) => HumanError {
            message: "There was a security problem.".into(),
            suggestion: "The app's secure storage may need to be reset. Go to Settings and try clearing the security data.".into(),
            retriable: false,
            severity: Severity::Permanent,
        },

        PresswerkError::IntegrityMismatch { .. } => HumanError {
            message: "This file has been changed since it was stored.".into(),
            suggestion: "The stored copy doesn't match the original. Try loading the file again from the original source.".into(),
            retriable: false,
            severity: Severity::Permanent,
        },

        PresswerkError::Certificate(_) => HumanError {
            message: "Secure connection setup failed.".into(),
            suggestion: "Try restarting the app. If this keeps happening, the security certificates may need to be regenerated in Settings.".into(),
            retriable: true,
            severity: Severity::Transient,
        },

        // -- Storage --
        PresswerkError::Database(_) => HumanError {
            message: "The app's data storage had a problem.".into(),
            suggestion: "Try closing and reopening the app. Your print jobs should still be there.".into(),
            retriable: true,
            severity: Severity::Transient,
        },

        PresswerkError::Io(io_err) => {
            if io_err.kind() == std::io::ErrorKind::NotFound {
                HumanError {
                    message: "The file couldn't be found.".into(),
                    suggestion: "It may have been moved or deleted. Try choosing the file again.".into(),
                    retriable: false,
                    severity: Severity::ActionRequired,
                }
            } else if io_err.kind() == std::io::ErrorKind::PermissionDenied {
                HumanError {
                    message: "The app doesn't have permission to read that file.".into(),
                    suggestion: "Check the file permissions, or try copying the file to a different location first.".into(),
                    retriable: false,
                    severity: Severity::ActionRequired,
                }
            } else {
                HumanError {
                    message: "There was a problem reading or writing a file.".into(),
                    suggestion: "Try again. If this keeps happening, your device's storage may be full.".into(),
                    retriable: true,
                    severity: Severity::Transient,
                }
            }
        }

        PresswerkError::Serialization(_) => HumanError {
            message: "The app had an internal data problem.".into(),
            suggestion: "Try again. If this keeps happening, please report it.".into(),
            retriable: true,
            severity: Severity::Transient,
        },

        // -- Platform --
        PresswerkError::Bridge(_) => HumanError {
            message: "A device-specific feature didn't work.".into(),
            suggestion: "Try restarting the app. Some features may not be available on all devices.".into(),
            retriable: true,
            severity: Severity::Transient,
        },

        PresswerkError::PlatformUnavailable => HumanError {
            message: "This feature isn't available on your device.".into(),
            suggestion: "Some features require a specific type of phone or tablet.".into(),
            retriable: false,
            severity: Severity::Permanent,
        },
    }
}

/// Parse IPP-specific error details into human-readable messages.
fn humanize_ipp_error(detail: &str) -> HumanError {
    let lower = detail.to_ascii_lowercase();

    if lower.contains("timed out") {
        HumanError {
            message: "The printer didn't respond in time.".into(),
            suggestion: "The printer might be busy or turned off. Check it's on and connected, then try again.".into(),
            retriable: true,
            severity: Severity::Transient,
        }
    } else if lower.contains("connection refused") {
        HumanError {
            message: "The printer refused our connection.".into(),
            suggestion: "The printer may be turned off, busy, or not accepting network connections. Try turning it off and on again.".into(),
            retriable: true,
            severity: Severity::Transient,
        }
    } else if lower.contains("connection reset") || lower.contains("broken pipe") {
        HumanError {
            message: "The connection to the printer was interrupted.".into(),
            suggestion: "This sometimes happens with Wi-Fi. We'll try again automatically.".into(),
            retriable: true,
            severity: Severity::Transient,
        }
    } else if lower.contains("server-error") {
        HumanError {
            message: "The printer reported an internal error.".into(),
            suggestion: "Try turning the printer off, waiting 10 seconds, and turning it back on."
                .into(),
            retriable: true,
            severity: Severity::Transient,
        }
    } else if lower.contains("client-error-not-possible")
        || lower.contains("client-error-attributes")
    {
        HumanError {
            message: "The printer can't handle those settings.".into(),
            suggestion:
                "Try changing the print settings (paper size, duplex, colour) and print again."
                    .into(),
            retriable: false,
            severity: Severity::ActionRequired,
        }
    } else if lower.contains("client-error-document-format") {
        HumanError {
            message: "The printer doesn't understand this file type.".into(),
            suggestion: "Try saving the file as a PDF first, then print the PDF.".into(),
            retriable: false,
            severity: Severity::Permanent,
        }
    } else if lower.contains("invalid uri") || lower.contains("invalid url") {
        HumanError {
            message: "The printer address doesn't look right.".into(),
            suggestion:
                "Check the printer address and try again. It should look like 192.168.1.100.".into(),
            retriable: false,
            severity: Severity::ActionRequired,
        }
    } else if lower.contains("media-empty") || lower.contains("out of paper") {
        HumanError {
            message: "The printer is out of paper.".into(),
            suggestion: "Please add paper to the printer's tray, then tap Retry.".into(),
            retriable: false,
            severity: Severity::ActionRequired,
        }
    } else if lower.contains("toner-empty")
        || lower.contains("ink")
        || lower.contains("marker-supply")
    {
        HumanError {
            message: "The printer needs new ink or toner.".into(),
            suggestion: "You'll need to buy a replacement cartridge. Check your printer's model number and search online for the right one.".into(),
            retriable: false,
            severity: Severity::BuyRequired,
        }
    } else if lower.contains("door-open") || lower.contains("cover-open") {
        HumanError {
            message: "A door or cover is open on the printer.".into(),
            suggestion: "Please close all doors and covers on the printer, then tap Retry.".into(),
            retriable: false,
            severity: Severity::ActionRequired,
        }
    } else if lower.contains("paper-jam") || lower.contains("media-jam") {
        HumanError {
            message: "Paper is stuck in the printer.".into(),
            suggestion: "Gently pull the stuck paper out. Check there are no torn pieces left inside, then close all doors.".into(),
            retriable: false,
            severity: Severity::ActionRequired,
        }
    } else {
        // Generic IPP error fallback
        HumanError {
            message: "The printer had a problem.".into(),
            suggestion: format!(
                "Try again. If this keeps happening, try turning the printer off and on again. (Detail: {detail})"
            ),
            retriable: true,
            severity: Severity::Transient,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeout_is_transient() {
        let err = PresswerkError::IppRequest("Get-Printer-Attributes timed out after 15s".into());
        let human = humanize_error(&err);
        assert_eq!(human.severity, Severity::Transient);
        assert!(human.retriable);
    }

    #[test]
    fn no_printer_is_action_required() {
        let human = humanize_error(&PresswerkError::NoPrinterSelected);
        assert_eq!(human.severity, Severity::ActionRequired);
        assert!(!human.retriable);
    }

    #[test]
    fn ink_empty_is_buy_required() {
        let err = PresswerkError::IppRequest("printer stopped: toner-empty".into());
        let human = humanize_error(&err);
        assert_eq!(human.severity, Severity::BuyRequired);
    }

    #[test]
    fn paper_jam_is_action_required() {
        let err = PresswerkError::IppRequest("printer stopped: media-jam".into());
        let human = humanize_error(&err);
        assert_eq!(human.severity, Severity::ActionRequired);
    }

    #[test]
    fn unsupported_format_is_permanent() {
        let err = PresswerkError::UnsupportedDocument("application/msword".into());
        let human = humanize_error(&err);
        assert_eq!(human.severity, Severity::Permanent);
    }
}

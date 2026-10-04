// SPDX-License-Identifier: MPL-2.0
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>
//
// FFP — Form-Fill Provenance, v1.0.0
// Normative types for the print path (issue #118, ruling D189).
// Wire vocabulary `ffp = "1.0"` is defined here; see
// 1-formats/sub-specs/form-fill-provenance/ (spec).

use serde::{Deserialize, Serialize};

/// Classification lattice (DETECTION §6, README lattice).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum FfpClassification {
    /// No interactive form or no fillable terminal field.
    #[serde(rename = "no-form")]
    NoForm,
    /// Form present, zero meaningful values.
    #[serde(rename = "blank-form")]
    BlankForm,
    /// ≥1 meaningful value and FFP marker declares `filledBy=machine`.
    #[serde(rename = "machine-filled")]
    MachineFilled,
    /// ≥1 meaningful value, no marker, NeedAppearances && incomplete appearances.
    #[serde(rename = "machine-filled-suspected")]
    MachineFilledSuspected,
    /// ≥1 meaningful value, none of the above.
    #[serde(rename = "filled-unknown")]
    FilledUnknown,
    /// Could not be parsed.
    #[default]
    #[serde(rename = "unreadable")]
    Unreadable,
}

impl FfpClassification {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::NoForm => "no-form",
            Self::BlankForm => "blank-form",
            Self::MachineFilled => "machine-filled",
            Self::MachineFilledSuspected => "machine-filled-suspected",
            Self::FilledUnknown => "filled-unknown",
            Self::Unreadable => "unreadable",
        }
    }

    // Inherent `from_str` (token vocabulary pair to `as_str`), deliberately
    // not `std::str::FromStr`: parsing never fails — unrecognised tokens
    // surface as `Unreadable` rather than as an error, matching
    // `FormProvenancePolicy::from_token`.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Self {
        match s {
            "no-form" => Self::NoForm,
            "blank-form" => Self::BlankForm,
            "machine-filled" => Self::MachineFilled,
            "machine-filled-suspected" => Self::MachineFilledSuspected,
            "filled-unknown" => Self::FilledUnknown,
            "unreadable" => Self::Unreadable,
            _ => Self::Unreadable,
        }
    }

    /// Whether the job is machine-filled (declared).
    pub fn is_machine(&self) -> bool {
        matches!(self, Self::MachineFilled)
    }

    /// Whether the job is machine-filled or suspected (structural).
    pub fn is_machine_or_suspected(&self) -> bool {
        matches!(self, Self::MachineFilled | Self::MachineFilledSuspected)
    }
}

impl std::fmt::Display for FfpClassification {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Form presence (DETECTION record `form`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum FfpForm {
    #[serde(rename = "present")]
    Present,
    #[serde(rename = "absent")]
    Absent,
    #[default]
    #[serde(rename = "unknown")]
    Unknown,
}

impl FfpForm {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Present => "present",
            Self::Absent => "absent",
            Self::Unknown => "unknown",
        }
    }
}

impl std::fmt::Display for FfpForm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Appearance state (DETECTION §5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum FfpAppearances {
    #[serde(rename = "generated")]
    Generated,
    #[serde(rename = "incomplete")]
    Incomplete,
    #[serde(rename = "not-applicable")]
    NotApplicable,
    #[default]
    #[serde(rename = "unknown")]
    Unknown,
}

impl FfpAppearances {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Generated => "generated",
            Self::Incomplete => "incomplete",
            Self::NotApplicable => "not-applicable",
            Self::Unknown => "unknown",
        }
    }
}

impl std::fmt::Display for FfpAppearances {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Declared marker (MARKER.adoc).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct FfpDeclared {
    /// Verbatim `filledBy` value.
    #[serde(rename = "filledBy")]
    pub filled_by: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    #[serde(rename = "toolVersion", skip_serializing_if = "Option::is_none")]
    pub tool_version: Option<String>,
    #[serde(rename = "filledAt", skip_serializing_if = "Option::is_none")]
    pub filled_at: Option<String>,
    #[serde(
        rename = "appearancesGenerated",
        skip_serializing_if = "Option::is_none"
    )]
    pub appearances_generated: Option<bool>,
}

impl FfpDeclared {
    pub fn machine(
        tool: Option<String>,
        tool_version: Option<String>,
        filled_at: Option<String>,
        appearances_generated: Option<bool>,
    ) -> Self {
        Self {
            filled_by: "machine".to_string(),
            tool,
            tool_version,
            filled_at,
            appearances_generated,
        }
    }
}

/// The FFP record (DETECTION §The record). Wire version `ffp = "1.0"`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FfpRecord {
    /// Wire version, always `"1.0"`.
    pub ffp: String,
    pub classification: FfpClassification,
    pub form: FfpForm,
    #[serde(rename = "filled_fields")]
    pub filled_fields: u32,
    #[serde(rename = "total_fields")]
    pub total_fields: u32,
    pub appearances: FfpAppearances,
    /// `null` when no marker parsed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub declared: Option<FfpDeclared>,
    /// Evidence codes, sorted, deduped.
    pub evidence: Vec<String>,
}

impl Default for FfpRecord {
    fn default() -> Self {
        Self::unreadable("not inspected")
    }
}

impl FfpRecord {
    /// Create an `unreadable` record.
    pub fn unreadable(reason: &str) -> Self {
        let _ = reason;
        Self {
            ffp: "1.0".to_string(),
            classification: FfpClassification::Unreadable,
            form: FfpForm::Unknown,
            filled_fields: 0,
            total_fields: 0,
            appearances: FfpAppearances::Unknown,
            declared: None,
            evidence: vec!["FFP-E-UNREADABLE".to_string()],
        }
    }

    /// Create a `no-form` record.
    pub fn no_form(evidence: Vec<String>) -> Self {
        let mut ev = evidence;
        if !ev.contains(&"FFP-E-NO-ACROFORM".to_string()) {
            ev.push("FFP-E-NO-ACROFORM".to_string());
        }
        ev.sort();
        ev.dedup();
        Self {
            ffp: "1.0".to_string(),
            classification: FfpClassification::NoForm,
            form: FfpForm::Absent,
            filled_fields: 0,
            total_fields: 0,
            appearances: FfpAppearances::NotApplicable,
            declared: None,
            evidence: ev,
        }
    }

    /// Whether the document carries an interactive form.
    pub fn is_form(&self) -> bool {
        matches!(self.form, FfpForm::Present)
    }

    /// Whether the form was declared machine-filled.
    pub fn is_machine_filled(&self) -> bool {
        self.classification == FfpClassification::MachineFilled
    }

    /// Whether the form is machine-filled or suspected.
    pub fn is_machine_or_suspected(&self) -> bool {
        self.classification.is_machine_or_suspected()
    }

    /// Whether the record indicates the blank-print hazard (P5):
    /// appearances incomplete and at least one filled field.
    pub fn has_blank_print_hazard(&self) -> bool {
        self.appearances == FfpAppearances::Incomplete && self.filled_fields > 0
    }

    /// Whether a job carrying this record should be held under `policy`.
    pub fn should_hold(&self, policy: crate::types::FormProvenancePolicy) -> bool {
        use crate::types::FormProvenancePolicy;
        match policy {
            FormProvenancePolicy::HoldForReview => {
                self.is_machine_or_suspected() || self.has_blank_print_hazard()
            }
            _ => false,
        }
    }

    /// One-line human summary for UI/logs/audit.
    pub fn summary(&self) -> String {
        let head = match self.classification {
            FfpClassification::NoForm => "not an interactive form".to_string(),
            FfpClassification::BlankForm => "blank form — no values entered".to_string(),
            FfpClassification::MachineFilled => {
                if let Some(d) = &self.declared {
                    if let Some(tool) = &d.tool {
                        format!("machine-filled form ({tool})")
                    } else {
                        "machine-filled form (declared)".to_string()
                    }
                } else {
                    "machine-filled form".to_string()
                }
            }
            FfpClassification::MachineFilledSuspected => {
                "machine-filled form (suspected — incomplete appearances)".to_string()
            }
            FfpClassification::FilledUnknown => {
                "form values present — provenance unknown".to_string()
            }
            FfpClassification::Unreadable => "form provenance unreadable".to_string(),
        };
        if self.is_form() {
            format!(
                "{head}; {}/{} fields filled; appearances {}",
                self.filled_fields,
                self.total_fields,
                self.appearances.as_str()
            )
        } else {
            head
        }
    }

    /// Compact JSON for audit trail.
    pub fn audit_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{}".to_string())
    }

    /// Canonical line for conformance runner:
    /// `classification=… form=… filled=N/M appearances=… evidence=…`
    pub fn canonical_line(&self) -> String {
        let mut ev = self.evidence.clone();
        ev.sort();
        ev.dedup();
        let evidence = ev.join(",");
        format!(
            "classification={} form={} filled={}/{} appearances={} evidence={}",
            self.classification.as_str(),
            self.form.as_str(),
            self.filled_fields,
            self.total_fields,
            self.appearances.as_str(),
            evidence
        )
    }

    /// Short label for UI badges (P4).
    pub fn label(&self) -> &'static str {
        match self.classification {
            FfpClassification::NoForm => "No form",
            FfpClassification::BlankForm => "Blank form",
            FfpClassification::MachineFilled => "Machine-filled",
            FfpClassification::MachineFilledSuspected => "Machine-filled (suspected)",
            FfpClassification::FilledUnknown => "Filled",
            FfpClassification::Unreadable => "Unreadable",
        }
    }

    // --- Compatibility shims for old code that used `origin`/`confidence`/`inspected` ---

    /// Compatibility: `origin` alias for `classification` (old `FormOrigin` tokens).
    /// Old tokens: NotAForm/Empty/Human/Machine/Unknown → new lattice.
    pub fn origin_token(&self) -> &'static str {
        match self.classification {
            FfpClassification::NoForm => "NotAForm",
            FfpClassification::BlankForm => "Empty",
            FfpClassification::FilledUnknown => "Human",
            FfpClassification::MachineFilled | FfpClassification::MachineFilledSuspected => {
                "Machine"
            }
            FfpClassification::Unreadable => "Unknown",
        }
    }

    /// Compatibility: `inspected` — false only for unreadable due to disabled policy.
    pub fn inspected(&self) -> bool {
        !matches!(self.classification, FfpClassification::Unreadable)
    }

    /// Compatibility: `field_count` alias.
    pub fn field_count(&self) -> u32 {
        self.total_fields
    }
    pub fn filled_field_count(&self) -> u32 {
        self.filled_fields
    }
}

// --- Legacy types for DB migration (pre-FFP-1.0) ------------------------
// These are retained ONLY to deserialize rows written before 2026-10-04.
// New code must use `Ffp*`. Do not construct these except in migration.

/// Legacy origin (pre-1.0). Kept for `form_origin` column migration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum FormOrigin {
    NotAForm,
    Empty,
    Human,
    Machine,
    #[default]
    Unknown,
}

impl FormOrigin {
    pub fn as_token(&self) -> &'static str {
        match self {
            Self::NotAForm => "NotAForm",
            Self::Empty => "Empty",
            Self::Human => "Human",
            Self::Machine => "Machine",
            Self::Unknown => "Unknown",
        }
    }
    pub fn from_token(token: &str) -> Self {
        match token {
            "NotAForm" => Self::NotAForm,
            "Empty" => Self::Empty,
            "Human" => Self::Human,
            "Machine" => Self::Machine,
            _ => Self::Unknown,
        }
    }
    pub fn is_machine(&self) -> bool {
        matches!(self, Self::Machine)
    }
}

impl std::fmt::Display for FormOrigin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_token())
    }
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default,
)]
pub enum ProvenanceConfidence {
    #[default]
    None,
    Speculative,
    Probable,
    Strong,
    Explicit,
}

impl ProvenanceConfidence {
    pub fn as_token(&self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Speculative => "Speculative",
            Self::Probable => "Probable",
            Self::Strong => "Strong",
            Self::Explicit => "Explicit",
        }
    }
}
impl std::fmt::Display for ProvenanceConfidence {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_token())
    }
}

/// Legacy provenance struct (pre-1.0). Only for deserializing old JSON.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FormProvenanceLegacy {
    pub origin: FormOrigin,
    pub confidence: ProvenanceConfidence,
    pub inspected: bool,
    pub is_form: bool,
    pub field_count: u32,
    pub filled_field_count: u32,
    pub values_without_appearance: u32,
    pub filled_with_default: u32,
    pub need_appearances: bool,
    pub producer: Option<String>,
    pub creator: Option<String>,
    pub marker: Option<String>,
    pub evidence: Vec<String>,
}

impl Default for FormProvenanceLegacy {
    fn default() -> Self {
        Self {
            origin: FormOrigin::Unknown,
            confidence: ProvenanceConfidence::None,
            inspected: false,
            is_form: false,
            field_count: 0,
            filled_field_count: 0,
            values_without_appearance: 0,
            filled_with_default: 0,
            need_appearances: false,
            producer: None,
            creator: None,
            marker: None,
            evidence: vec!["document was not inspected".to_string()],
        }
    }
}

/// New code uses `FfpRecord`. This alias keeps `crate::FormProvenance` compiling
/// for call sites that have migrated; it points to the spec type.
pub type FormProvenance = FfpRecord;

/// Convert legacy into new record (best-effort, for migration).
impl From<FormProvenanceLegacy> for FfpRecord {
    fn from(legacy: FormProvenanceLegacy) -> Self {
        // Map legacy origin → new classification
        let classification = match legacy.origin {
            FormOrigin::NotAForm => FfpClassification::NoForm,
            FormOrigin::Empty => FfpClassification::BlankForm,
            FormOrigin::Human => FfpClassification::FilledUnknown,
            FormOrigin::Machine => {
                // Old Machine covered both declared and suspected; map to suspected
                // unless marker indicated explicit.
                if legacy.marker.is_some() {
                    FfpClassification::MachineFilled
                } else {
                    FfpClassification::MachineFilledSuspected
                }
            }
            FormOrigin::Unknown => FfpClassification::Unreadable,
        };
        let form = if legacy.is_form {
            FfpForm::Present
        } else if legacy.origin == FormOrigin::NotAForm {
            FfpForm::Absent
        } else if legacy.origin == FormOrigin::Unknown && !legacy.inspected {
            FfpForm::Unknown
        } else {
            FfpForm::Absent
        };
        let appearances = if legacy.filled_field_count == 0 {
            FfpAppearances::NotApplicable
        } else if legacy.values_without_appearance > 0 || legacy.need_appearances {
            FfpAppearances::Incomplete
        } else {
            FfpAppearances::Generated
        };
        let declared = if let Some(m) = &legacy.marker {
            Some(FfpDeclared {
                filled_by: m.clone(),
                tool: legacy.producer.clone(),
                tool_version: None,
                filled_at: None,
                appearances_generated: None,
            })
        } else {
            legacy.producer.as_ref().map(|p| FfpDeclared {
                filled_by: "machine".to_string(),
                tool: Some(p.clone()),
                tool_version: None,
                filled_at: None,
                appearances_generated: None,
            })
        };
        let mut evidence = Vec::new();
        if legacy.need_appearances {
            evidence.push("FFP-E-NEED-APPEARANCES".to_string());
        }
        if legacy.values_without_appearance > 0 {
            evidence.push("FFP-E-AP-INCOMPLETE".to_string());
        }
        match legacy.origin {
            FormOrigin::NotAForm => evidence.push("FFP-E-NO-ACROFORM".to_string()),
            FormOrigin::Empty => evidence.push("FFP-E-NO-VALUES".to_string()),
            FormOrigin::Unknown if !legacy.inspected => {
                evidence.push("FFP-E-UNREADABLE".to_string())
            }
            _ => {}
        }
        if legacy.marker.is_some() {
            evidence.push("FFP-E-DECL-MACHINE".to_string());
        }
        if evidence.is_empty() && legacy.evidence.iter().any(|e| e.contains("not inspected")) {
            evidence.push("FFP-E-UNREADABLE".to_string());
        }
        evidence.sort();
        evidence.dedup();
        if evidence.is_empty() {
            evidence = legacy.evidence;
        }
        FfpRecord {
            ffp: "1.0".to_string(),
            classification,
            form,
            filled_fields: legacy.filled_field_count,
            total_fields: legacy.field_count,
            appearances,
            declared,
            evidence,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classification_tokens_roundtrip() {
        for c in [
            FfpClassification::NoForm,
            FfpClassification::BlankForm,
            FfpClassification::MachineFilled,
            FfpClassification::MachineFilledSuspected,
            FfpClassification::FilledUnknown,
            FfpClassification::Unreadable,
        ] {
            let s = c.as_str();
            assert_eq!(FfpClassification::from_str(s), c);
            assert!(!s.is_empty());
        }
    }

    #[test]
    fn canonical_line_format() {
        let r = FfpRecord {
            ffp: "1.0".to_string(),
            classification: FfpClassification::MachineFilledSuspected,
            form: FfpForm::Present,
            filled_fields: 2,
            total_fields: 2,
            appearances: FfpAppearances::Incomplete,
            declared: None,
            evidence: vec![
                "FFP-E-NEED-APPEARANCES".to_string(),
                "FFP-E-AP-INCOMPLETE".to_string(),
            ],
        };
        assert_eq!(
            r.canonical_line(),
            "classification=machine-filled-suspected form=present filled=2/2 appearances=incomplete evidence=FFP-E-AP-INCOMPLETE,FFP-E-NEED-APPEARANCES"
        );
        // evidence is sorted
    }

    #[test]
    fn json_roundtrip() {
        let r = FfpRecord {
            ffp: "1.0".to_string(),
            classification: FfpClassification::MachineFilled,
            form: FfpForm::Present,
            filled_fields: 2,
            total_fields: 2,
            appearances: FfpAppearances::Incomplete,
            declared: Some(FfpDeclared {
                filled_by: "machine".to_string(),
                tool: Some("blocky-writer".to_string()),
                tool_version: Some("0.2.0".to_string()),
                filled_at: None,
                appearances_generated: Some(false),
            }),
            evidence: vec!["FFP-E-DECL-MACHINE".to_string()],
        };
        let json = r.audit_json();
        let back: FfpRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(back, r);
    }

    #[test]
    fn unreadable_line() {
        let r = FfpRecord::unreadable("parse failed");
        assert_eq!(
            r.canonical_line(),
            "classification=unreadable form=unknown filled=0/0 appearances=unknown evidence=FFP-E-UNREADABLE"
        );
    }

    #[test]
    fn should_hold() {
        use crate::types::FormProvenancePolicy;
        let mut r = FfpRecord {
            ffp: "1.0".to_string(),
            classification: FfpClassification::MachineFilled,
            form: FfpForm::Present,
            filled_fields: 2,
            total_fields: 2,
            appearances: FfpAppearances::Incomplete,
            declared: Some(FfpDeclared {
                filled_by: "machine".to_string(),
                tool: None,
                tool_version: None,
                filled_at: None,
                appearances_generated: None,
            }),
            evidence: vec!["FFP-E-DECL-MACHINE".to_string()],
        };
        assert!(r.should_hold(FormProvenancePolicy::HoldForReview));
        r.classification = FfpClassification::MachineFilledSuspected;
        assert!(r.should_hold(FormProvenancePolicy::HoldForReview));
        r.classification = FfpClassification::FilledUnknown;
        // FilledUnknown with incomplete appearances is still hazard -> hold
        r.appearances = FfpAppearances::Incomplete;
        assert!(r.should_hold(FormProvenancePolicy::HoldForReview));
        r.appearances = FfpAppearances::Generated;
        assert!(!r.should_hold(FormProvenancePolicy::HoldForReview));
        assert!(!r.should_hold(FormProvenancePolicy::Record));
    }
}

// SPDX-License-Identifier: MPL-2.0
// Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>
//
// Service layer — bridges the Dioxus UI to the presswerk backend crates.
//
// Each service wraps one or more backend crate APIs in a way that is convenient
// for the UI to call (async-friendly, returns data the UI can display directly).

pub mod app_services;
pub mod data_dir;

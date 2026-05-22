||| Presswerk ABI Protocol Proofs
|||
||| Formal verification of IPP protocol correctness and job state machine.
|||
||| SPDX-License-Identifier: MPL-2.0
||| Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath)

module Presswerk.ABI.Protocol

import Presswerk.ABI.Types

%default total

--------------------------------------------------------------------------------
-- IPP Operation Codes (RFC 8011 Section 4.4)
--------------------------------------------------------------------------------

||| IPP operation identifiers
public export
data IppOp
  = PrintJob           -- 0x0002
  | ValidateJob        -- 0x0004
  | CancelJob          -- 0x0008
  | GetJobAttributes   -- 0x0009
  | GetJobs            -- 0x000A
  | GetPrinterAttrs    -- 0x000B

||| Map IPP ops to their RFC-defined code points
public export
ippOpCode : IppOp -> Bits16
ippOpCode PrintJob         = 0x0002
ippOpCode ValidateJob      = 0x0004
ippOpCode CancelJob        = 0x0008
ippOpCode GetJobAttributes = 0x0009
ippOpCode GetJobs          = 0x000A
ippOpCode GetPrinterAttrs  = 0x000B

||| Proof: ippOpCode is injective (no two ops share a code)
public export
ippOpInjective : (a, b : IppOp) -> ippOpCode a = ippOpCode b -> a = b
ippOpInjective PrintJob         PrintJob         Refl = Refl
ippOpInjective ValidateJob      ValidateJob      Refl = Refl
ippOpInjective CancelJob        CancelJob        Refl = Refl
ippOpInjective GetJobAttributes GetJobAttributes Refl = Refl
ippOpInjective GetJobs          GetJobs          Refl = Refl
ippOpInjective GetPrinterAttrs  GetPrinterAttrs  Refl = Refl

--------------------------------------------------------------------------------
-- Job State Machine
--------------------------------------------------------------------------------

||| Valid transitions in the print job state machine.
||| Only these transitions are permitted — all others are type errors.
public export
data ValidTransition : JobStatus -> JobStatus -> Type where
  ||| Pending -> Processing (job picked up by printer)
  StartProcessing  : ValidTransition Pending Processing
  ||| Processing -> Completed (print succeeded)
  Complete         : ValidTransition Processing Completed
  ||| Processing -> Failed (print error)
  Fail             : ValidTransition Processing Failed
  ||| Pending -> Cancelled (user cancelled before processing)
  CancelPending    : ValidTransition Pending Cancelled
  ||| Processing -> Cancelled (user cancelled during processing)
  CancelProcessing : ValidTransition Processing Cancelled
  ||| Pending -> Held (network job held for review)
  HoldPending      : ValidTransition Pending Held
  ||| Held -> Pending (user approved held job)
  ReleasePending   : ValidTransition Held Pending
  ||| Held -> Cancelled (user rejected held job)
  CancelHeld       : ValidTransition Held Cancelled

||| Terminal states — no further transitions possible
public export
data IsTerminal : JobStatus -> Type where
  CompletedIsTerminal : IsTerminal Completed
  FailedIsTerminal    : IsTerminal Failed
  CancelledIsTerminal : IsTerminal Cancelled

||| Proof: Completed is terminal (cannot transition out)
public export
completedNoExit : ValidTransition Completed s -> Void
completedNoExit _ impossible

||| Proof: Failed is terminal
public export
failedNoExit : ValidTransition Failed s -> Void
failedNoExit _ impossible

||| Proof: Cancelled is terminal
public export
cancelledNoExit : ValidTransition Cancelled s -> Void
cancelledNoExit _ impossible

||| A sequence of valid transitions (reachability proof)
public export
data Reachable : JobStatus -> JobStatus -> Type where
  Here  : Reachable s s
  There : ValidTransition s t -> Reachable t u -> Reachable s u

||| Proof: Completed is reachable from Pending
public export
pendingToCompleted : Reachable Pending Completed
pendingToCompleted = There StartProcessing (There Complete Here)

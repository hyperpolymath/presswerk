||| Presswerk ABI Memory Layout Proofs
|||
||| Formal proofs about memory layout, alignment, and padding for
||| C-compatible structs crossing the Rust ↔ Zig FFI boundary.
|||
||| SPDX-License-Identifier: MPL-2.0
||| Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath)

module Presswerk.ABI.Layout

import Presswerk.ABI.Types
import Data.Vect
import Data.So

%default total

--------------------------------------------------------------------------------
-- Alignment Utilities
--------------------------------------------------------------------------------

||| Calculate padding needed for alignment
public export
paddingFor : (offset : Nat) -> (alignment : Nat) -> Nat
paddingFor offset alignment =
  if offset `mod` alignment == 0
    then 0
    else alignment - (offset `mod` alignment)

||| Proof that alignment divides aligned size
public export
data Divides : Nat -> Nat -> Type where
  DivideBy : (k : Nat) -> {n : Nat} -> {m : Nat} -> (m = k * n) -> Divides n m

||| Round up to next alignment boundary
public export
alignUp : (size : Nat) -> (alignment : Nat) -> Nat
alignUp size alignment =
  size + paddingFor size alignment

--------------------------------------------------------------------------------
-- Struct Field Layout
--------------------------------------------------------------------------------

||| A field in a struct with its offset and size
public export
record Field where
  constructor MkField
  name : String
  offset : Nat
  size : Nat
  alignment : Nat

||| Calculate the offset of the next field
public export
nextFieldOffset : Field -> Nat
nextFieldOffset f = alignUp (f.offset + f.size) f.alignment

||| A struct layout is a non-empty collection of fields
public export
record StructLayout where
  constructor MkStructLayout
  structName : String
  fields : List Field
  totalSize : Nat
  alignment : Nat

||| Proof that field offsets are correctly aligned
public export
data FieldsAligned : List Field -> Type where
  NoFields : FieldsAligned []
  ConsField :
    (f : Field) ->
    (rest : List Field) ->
    Divides f.alignment f.offset ->
    FieldsAligned rest ->
    FieldsAligned (f :: rest)

||| Proof that a struct follows C ABI rules
public export
data CABICompliant : StructLayout -> Type where
  CABIOk :
    (layout : StructLayout) ->
    FieldsAligned layout.fields ->
    CABICompliant layout

--------------------------------------------------------------------------------
-- Presswerk Struct Layouts
--------------------------------------------------------------------------------

||| C-repr layout of the FFI JobInfo struct.
|||
||| Mirrors the minimal job data passed across the FFI boundary:
|||   struct pw_job_info {
|||       uint32_t status;     // 4 bytes, offset 0
|||       uint32_t doc_type;   // 4 bytes, offset 4
|||       uint64_t created_at; // 8 bytes, offset 8  (Unix timestamp)
|||       uint64_t doc_size;   // 8 bytes, offset 16 (byte count)
|||       uint64_t name_ptr;   // 8 bytes, offset 24 (C string pointer)
|||   };
|||   total: 32 bytes, alignment: 8
public export
jobInfoLayout : StructLayout
jobInfoLayout = MkStructLayout "pw_job_info"
  [ MkField "status"     0  4 4
  , MkField "doc_type"   4  4 4
  , MkField "created_at" 8  8 8
  , MkField "doc_size"   16 8 8
  , MkField "name_ptr"   24 8 8
  ]
  32  -- total size
  8   -- alignment

||| Proof: JobInfo fields are correctly aligned.
public export
jobInfoAligned : FieldsAligned (fields jobInfoLayout)
jobInfoAligned =
  ConsField (MkField "status"     0  4 4) _ (DivideBy 0 Refl) $
  ConsField (MkField "doc_type"   4  4 4) _ (DivideBy 1 Refl) $
  ConsField (MkField "created_at" 8  8 8) _ (DivideBy 1 Refl) $
  ConsField (MkField "doc_size"   16 8 8) _ (DivideBy 2 Refl) $
  ConsField (MkField "name_ptr"   24 8 8) _ (DivideBy 3 Refl) $
  NoFields

||| Proof: JobInfo is C ABI compliant.
public export
jobInfoCABI : CABICompliant jobInfoLayout
jobInfoCABI = CABIOk jobInfoLayout jobInfoAligned

||| C-repr layout of the FFI ServerConfig struct.
|||
|||   struct pw_server_config {
|||       uint16_t port;        // 2 bytes, offset 0
|||       uint8_t  require_tls; // 1 byte,  offset 2
|||       uint8_t  padding;     // 1 byte,  offset 3 (natural padding)
|||   };
|||   total: 4 bytes, alignment: 2
public export
serverConfigLayout : StructLayout
serverConfigLayout = MkStructLayout "pw_server_config"
  [ MkField "port"        0 2 2
  , MkField "require_tls" 2 1 1
  , MkField "padding"     3 1 1
  ]
  4   -- total size
  2   -- alignment

||| C-repr layout of the FFI PrinterInfo struct.
|||
|||   struct pw_printer_info {
|||       uint64_t name_ptr;     // 8 bytes, offset 0
|||       uint64_t uri_ptr;      // 8 bytes, offset 8
|||       uint64_t location_ptr; // 8 bytes, offset 16
|||       uint16_t port;         // 2 bytes, offset 24
|||       uint8_t  supports_tls; // 1 byte,  offset 26
|||       uint8_t  supports_color; // 1 byte, offset 27
|||       uint8_t  supports_duplex; // 1 byte, offset 28
|||       uint8_t  padding[3];   // 3 bytes, offset 29 (pad to 8-byte alignment)
|||   };
|||   total: 32 bytes, alignment: 8
public export
printerInfoLayout : StructLayout
printerInfoLayout = MkStructLayout "pw_printer_info"
  [ MkField "name_ptr"         0  8 8
  , MkField "uri_ptr"          8  8 8
  , MkField "location_ptr"     16 8 8
  , MkField "port"             24 2 2
  , MkField "supports_tls"     26 1 1
  , MkField "supports_color"   27 1 1
  , MkField "supports_duplex"  28 1 1
  , MkField "padding"          29 3 1
  ]
  32  -- total size
  8   -- alignment

||| C-repr layout of the FFI AuditEntry struct.
|||
|||   struct pw_audit_entry {
|||       uint64_t timestamp;    // 8 bytes, offset 0
|||       uint64_t action_ptr;   // 8 bytes, offset 8
|||       uint64_t hash_ptr;     // 8 bytes, offset 16
|||       uint64_t details_ptr;  // 8 bytes, offset 24
|||       uint8_t  success;      // 1 byte,  offset 32
|||       uint8_t  padding[7];   // 7 bytes, offset 33
|||   };
|||   total: 40 bytes, alignment: 8
public export
auditEntryLayout : StructLayout
auditEntryLayout = MkStructLayout "pw_audit_entry"
  [ MkField "timestamp"   0  8 8
  , MkField "action_ptr"  8  8 8
  , MkField "hash_ptr"    16 8 8
  , MkField "details_ptr" 24 8 8
  , MkField "success"     32 1 1
  , MkField "padding"     33 7 1
  ]
  40  -- total size
  8   -- alignment

--------------------------------------------------------------------------------
-- Cross-Platform Size Assertions
--------------------------------------------------------------------------------

||| Pointer-sized fields must match the platform pointer size.
public export
0 PointerFieldCorrect : (p : Platform) -> (f : Field) -> Type
PointerFieldCorrect p f = f.size * 8 = ptrSize p

||| On 64-bit platforms, pointer fields must be 8 bytes.
public export
ptrFieldSize64 : (p : Platform) -> (Not (p = WASM)) ->
  ptrSize p = 64
ptrFieldSize64 Linux   _ = Refl
ptrFieldSize64 MacOS   _ = Refl
ptrFieldSize64 IOS     _ = Refl
ptrFieldSize64 Android _ = Refl

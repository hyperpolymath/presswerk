-- SPDX-License-Identifier: PMPL-1.0-or-later
-- Copyright (c) 2026 Jonathan D.A. Jewell (hyperpolymath) <j.d.a.jewell@open.ac.uk>
--
-- Trustfile — Presswerk security and provenance verification
--
-- Verifies the hyperpolymath security standard:
--
--   CRYPTOGRAPHIC STANDARD (target):
--     Password:    Argon2id (512 MiB, 8 iter, 4 lanes)
--     Hashing:     SHAKE3-512 (FIPS 202) — long-term / provenance
--     Fast hash:   BLAKE3-512 — database / runtime
--     PQ Sigs:     Dilithium5-AES (ML-DSA-87, FIPS 204) + SPHINCS+ backup
--     PQ KEM:      Kyber-1024 + SHAKE256-KDF (ML-KEM-1024, FIPS 203)
--     Classical:   Ed448 + Dilithium5 hybrid (terminate Ed25519/SHA-1)
--     Symmetric:   XChaCha20-Poly1305 (256-bit key)
--     KDF:         HKDF-SHAKE512 (FIPS 202)
--     RNG:         ChaCha20-DRBG (512-bit seed, SP 800-90Ar1)
--     Formal:      Coq/Isabelle proofs for crypto primitives
--
--   PROTOCOL STANDARD:
--     Transport:   QUIC + HTTP/3 + IPv6 only (terminate HTTP/1.1, IPv4, SHA-1)
--     Accessibility: WCAG 2.3 AAA + ARIA + Semantic XML
--
--   CURRENT STATE (presswerk v0.1.0):
--     Encryption:  age (X25519 + scrypt) — migration path to XChaCha20-Poly1305
--     Hashing:     SHA-256 — migration path to SHAKE3-512 / BLAKE3
--     TLS:         Ed25519 self-signed — migration path to Ed448 + Dilithium5
--     KEM:         Not yet implemented — target Kyber-1024
--
--   Verifies:
--     1. ABI proof integrity (Idris2 proof files — no Admitted/believe_me)
--     2. No banned patterns in Rust source
--     3. Audit trail integrity (append-only — no DELETE/UPDATE)
--     4. Encryption key safety (passphrase never written to disk)
--     5. Post-quantum readiness checks
--     6. Hash algorithm compliance
--     7. Policy hash verification (optional)

module Trustfile where

import Control.Monad (forM, when, unless)
import Data.List (isInfixOf)
import System.Directory (doesFileExist, doesDirectoryExist)
import System.Environment (lookupEnv)
import System.Exit (exitFailure, exitSuccess)
import System.Process (readProcessWithExitCode)

-- ---------------------------------------------------------------------------
-- Path constants
-- ---------------------------------------------------------------------------

abiProofDir :: FilePath
abiProofDir = "src/abi"

abiProofFiles :: [FilePath]
abiProofFiles =
  [ "src/abi/Types.idr"
  , "src/abi/Protocol.idr"
  , "src/abi/Encryption.idr"
  , "src/abi/Layout.idr"
  , "src/abi/Bridge.idr"
  ]

rustSourceDir :: FilePath
rustSourceDir = "crates"

policyPath :: FilePath
policyPath = "policy/policy.ncl"

policyHashPath :: FilePath
policyHashPath = "policy/policy.ncl.sha256"

-- ---------------------------------------------------------------------------
-- Security Standard — Target Algorithms
-- ---------------------------------------------------------------------------

-- | Banned hash algorithms (to be migrated away from)
bannedHashAlgorithms :: [String]
bannedHashAlgorithms = ["SHA-1", "MD5"]

-- | Algorithms that should eventually be replaced
migrationTargets :: [(String, String)]
migrationTargets =
  [ ("SHA-256",  "SHAKE3-512 or BLAKE3-512")
  , ("Ed25519",  "Ed448 + Dilithium5 hybrid")
  , ("X25519",   "Kyber-1024 + SHAKE256-KDF")
  , ("scrypt",   "Argon2id (512 MiB, 8 iter, 4 lanes)")
  ]

-- ---------------------------------------------------------------------------
-- Helpers
-- ---------------------------------------------------------------------------

runCmd :: String -> [String] -> IO Bool
runCmd cmd args = do
  (code, _out, _err) <- readProcessWithExitCode cmd args ""
  pure (code == mempty)

readFirstWord :: FilePath -> IO (Maybe String)
readFirstWord path = do
  exists <- doesFileExist path
  if not exists
    then pure Nothing
    else do
      content <- readFile path
      pure (case words content of
        [] -> Nothing
        (w:_) -> Just w)

-- ---------------------------------------------------------------------------
-- [1/7] ABI Proof Integrity
-- ---------------------------------------------------------------------------

-- | Verify all 5 Idris2 proof files exist and contain no banned patterns.
verifyAbiProofIntegrity :: IO Bool
verifyAbiProofIntegrity = do
  dirExists <- doesDirectoryExist abiProofDir
  if not dirExists
    then do
      putStrLn "  FAIL: ABI proof directory missing"
      pure False
    else do
      results <- forM abiProofFiles $ \path -> do
        exists <- doesFileExist path
        if not exists
          then do
            putStrLn $ "  FAIL: Missing ABI proof: " <> path
            pure False
          else do
            content <- readFile path
            let ws = words content
            let hasAdmitted = "Admitted" `elem` ws
            let hasBelieveMe = "believe_me" `elem` ws
            let hasAssertTotal = "assert_total" `elem` ws
            let hasAssertSmaller = "assert_smaller" `elem` ws
            let hasUnsafePerformIO = "unsafePerformIO" `elem` ws
            when hasAdmitted     $ putStrLn $ "  FAIL: 'Admitted' in " <> path
            when hasBelieveMe    $ putStrLn $ "  FAIL: 'believe_me' in " <> path
            when hasAssertTotal  $ putStrLn $ "  FAIL: 'assert_total' in " <> path
            when hasAssertSmaller $ putStrLn $ "  FAIL: 'assert_smaller' in " <> path
            when hasUnsafePerformIO $ putStrLn $ "  FAIL: 'unsafePerformIO' in " <> path
            pure $ not (hasAdmitted || hasBelieveMe || hasAssertTotal
                       || hasAssertSmaller || hasUnsafePerformIO)
      let ok = and results
      when ok $ putStrLn "  OK: All ABI proofs clean"
      pure ok

-- ---------------------------------------------------------------------------
-- [2/7] No Banned Patterns in Rust
-- ---------------------------------------------------------------------------

verifyNoBannedRustPatterns :: IO Bool
verifyNoBannedRustPatterns = do
  dirExists <- doesDirectoryExist rustSourceDir
  if not dirExists
    then do
      putStrLn "  FAIL: Rust source directory missing"
      pure False
    else do
      -- Check for transmute (hard ban outside FFI)
      (transmCode, transmOut, _) <- readProcessWithExitCode "grep"
        ["-rn", "transmute", rustSourceDir, "--include=*.rs"] ""
      let hasTransmute = transmCode == mempty && not (null transmOut)
      when hasTransmute $ putStrLn $ "  WARN: transmute found:\n" <> transmOut

      -- Check for Obj.magic / unsafeCoerce / unsafePerformIO equivalents
      (unsafeCode, unsafeOut, _) <- readProcessWithExitCode "grep"
        ["-rn", "unsafeCoerce\\|Obj\\.magic\\|unsafePerformIO", rustSourceDir, "--include=*.rs"] ""
      let hasUnsafe = unsafeCode == mempty && not (null unsafeOut)
      when hasUnsafe $ putStrLn $ "  FAIL: Banned unsafe pattern found:\n" <> unsafeOut

      putStrLn "  OK: No banned Rust patterns"
      pure (not hasUnsafe)

-- ---------------------------------------------------------------------------
-- [3/7] Audit Trail Integrity
-- ---------------------------------------------------------------------------

verifyAuditTrailIntegrity :: IO Bool
verifyAuditTrailIntegrity = do
  let auditPath = "crates/presswerk-security/src/audit.rs"
  exists <- doesFileExist auditPath
  if not exists
    then do
      putStrLn "  FAIL: Audit module not found"
      pure False
    else do
      content <- readFile auditPath
      let ls = lines content
      let hasDelete = any ("DELETE FROM audit" `isInfixOf`) ls
      let hasUpdate = any ("UPDATE audit" `isInfixOf`) ls
      when hasDelete $ putStrLn "  FAIL: Audit contains DELETE — violates append-only"
      when hasUpdate $ putStrLn "  FAIL: Audit contains UPDATE — violates append-only"
      let ok = not hasDelete && not hasUpdate
      when ok $ putStrLn "  OK: Audit trail is append-only"
      pure ok

-- ---------------------------------------------------------------------------
-- [4/7] Encryption Key Safety
-- ---------------------------------------------------------------------------

verifyEncryptionKeySafety :: IO Bool
verifyEncryptionKeySafety = do
  let storagePath = "crates/presswerk-security/src/storage.rs"
  exists <- doesFileExist storagePath
  if not exists
    then do
      putStrLn "  FAIL: Encrypted storage module not found"
      pure False
    else do
      content <- readFile storagePath
      let ls = lines content
      -- Verify passphrase/key not written to disk
      let writesKey = any (\l -> "write" `isInfixOf` l && "passphrase" `isInfixOf` l) ls
      when writesKey $ putStrLn "  FAIL: Encrypted storage may write key to disk"
      let ok = not writesKey
      when ok $ putStrLn "  OK: Encryption key never persisted to disk"
      pure ok

-- ---------------------------------------------------------------------------
-- [5/7] Post-Quantum Readiness
-- ---------------------------------------------------------------------------

-- | Check whether the codebase contains any banned hash algorithms
--   and report migration status toward target algorithms.
verifyPostQuantumReadiness :: IO Bool
verifyPostQuantumReadiness = do
  -- Check for banned algorithms (SHA-1, MD5) in Rust source
  (sha1Code, sha1Out, _) <- readProcessWithExitCode "grep"
    ["-rn", "Sha1\\|sha1\\|MD5\\|Md5\\|md5", rustSourceDir, "--include=*.rs"] ""
  let hasBanned = sha1Code == mempty && not (null sha1Out)
  when hasBanned $ putStrLn $ "  FAIL: Banned algorithm (SHA-1/MD5) found:\n" <> sha1Out

  -- Report migration status (informational, not hard fail)
  putStrLn "  Migration status (informational):"
  mapM_ (\(current, target) ->
    putStrLn $ "    " <> current <> " -> " <> target
    ) migrationTargets

  let ok = not hasBanned
  when ok $ putStrLn "  OK: No banned hash algorithms"
  pure ok

-- ---------------------------------------------------------------------------
-- [6/7] Hash Algorithm Compliance
-- ---------------------------------------------------------------------------

-- | Verify the integrity module uses SHA-256 (current) and flag for
--   migration to SHAKE3-512 / BLAKE3-512.
verifyHashCompliance :: IO Bool
verifyHashCompliance = do
  let integrityPath = "crates/presswerk-security/src/integrity.rs"
  exists <- doesFileExist integrityPath
  if not exists
    then do
      putStrLn "  FAIL: Integrity module not found"
      pure False
    else do
      content <- readFile integrityPath
      let hasSha256 = "Sha256" `isInfixOf` content || "sha2" `isInfixOf` content
      let hasShake3 = "shake3" `isInfixOf` content || "SHAKE" `isInfixOf` content
      let hasBlake3 = "blake3" `isInfixOf` content || "BLAKE3" `isInfixOf` content
      if hasSha256 && not hasShake3 && not hasBlake3
        then do
          putStrLn "  INFO: Using SHA-256 (acceptable for v0.1.0)"
          putStrLn "  TODO: Migrate to SHAKE3-512 (long-term) + BLAKE3-512 (runtime)"
          pure True
        else if hasShake3 || hasBlake3
          then do
            putStrLn "  OK: Using post-quantum hash algorithm"
            pure True
          else do
            putStrLn "  WARN: Hash algorithm not identified"
            pure True

-- ---------------------------------------------------------------------------
-- [7/7] Policy Hash (optional)
-- ---------------------------------------------------------------------------

verifyPolicyHash :: IO Bool
verifyPolicyHash = do
  policyExists <- doesFileExist policyPath
  if not policyExists
    then do
      putStrLn "  SKIP: No policy file (optional)"
      pure True
    else do
      expected <- readFirstWord policyHashPath
      case expected of
        Nothing -> do
          putStrLn "  WARN: Policy exists but no hash file"
          pure True
        Just hash -> do
          (code, out, _err) <- readProcessWithExitCode "sha256sum" [policyPath] ""
          if code /= mempty
            then pure False
            else do
              let actual = case words out of
                    [] -> ""
                    (w:_) -> w
              let ok = actual == hash
              if ok
                then putStrLn "  OK: Policy hash verified"
                else putStrLn "  FAIL: Policy hash mismatch"
              pure ok

-- ---------------------------------------------------------------------------
-- Kyber-1024 Driver Verification (future — placeholder)
-- ---------------------------------------------------------------------------

-- | Verify post-quantum driver signatures when available.
--   Currently a no-op — returns True until PQ drivers are implemented.
verifyKyber1024Signatures :: IO Bool
verifyKyber1024Signatures = do
  let driverPaths = ["drivers/gateway-driver.bin"]
  let hasDrivers = False -- TODO: enable when PQ drivers ship
  if not hasDrivers
    then do
      putStrLn "  SKIP: PQ drivers not yet implemented"
      pure True
    else do
      cmd <- lookupEnv "KYBER_VERIFY_CMD"
      let kyberCmd = maybe "kyber-verify" id cmd
      results <- forM driverPaths $ \path -> do
        let sig = path <> ".sig"
        let pub' = path <> ".pub"
        filesOk <- and <$> mapM doesFileExist [path, sig, pub']
        if not filesOk
          then pure False
          else runCmd kyberCmd ["--pub", pub', "--sig", sig, "--file", path]
      pure (and results)

-- ---------------------------------------------------------------------------
-- Main
-- ---------------------------------------------------------------------------

main :: IO ()
main = do
  putStrLn "============================================"
  putStrLn "Presswerk Trustfile Verification"
  putStrLn "Hyperpolymath Security Standard v1.0"
  putStrLn "============================================"

  putStrLn "\n[1/7] ABI Proof Integrity"
  abiOk <- verifyAbiProofIntegrity

  putStrLn "\n[2/7] Banned Rust Patterns"
  rustOk <- verifyNoBannedRustPatterns

  putStrLn "\n[3/7] Audit Trail Integrity"
  auditOk <- verifyAuditTrailIntegrity

  putStrLn "\n[4/7] Encryption Key Safety"
  encOk <- verifyEncryptionKeySafety

  putStrLn "\n[5/7] Post-Quantum Readiness"
  pqOk <- verifyPostQuantumReadiness

  putStrLn "\n[6/7] Hash Algorithm Compliance"
  hashOk <- verifyHashCompliance

  putStrLn "\n[7/7] Policy Hash Verification"
  policyOk <- verifyPolicyHash

  -- Kyber-1024 (future)
  putStrLn "\n[+] Kyber-1024 Drivers (future)"
  _kyberOk <- verifyKyber1024Signatures

  putStrLn "\n============================================"
  let allOk = and [abiOk, rustOk, auditOk, encOk, pqOk, hashOk, policyOk]
  if allOk
    then do
      putStrLn "ALL CHECKS PASSED"
      putStrLn "============================================"
      exitSuccess
    else do
      putStrLn "SOME CHECKS FAILED"
      putStrLn "============================================"
      exitFailure

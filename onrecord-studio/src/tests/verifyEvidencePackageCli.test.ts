import { afterEach, describe, expect, it } from "vitest";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { mkdtemp, rm, symlink, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { webcrypto } from "node:crypto";

// Runs the real CLI as a subprocess end to end — this is the only layer that can catch a wiring
// bug like main() forgetting to pass `revocations` through to `verifyOne` (the exact bug found
// while building this feature): a unit test of `verifyChain` in isolation cannot see that class
// of bug, since the function it tests was never wrong: the caller not calling it correctly was.
const run = promisify(execFile);
const scriptPath = join(import.meta.dirname, "../../scripts/verify-evidence-package.mjs");

let tempDir: string | undefined;
afterEach(async () => {
  if (tempDir) await rm(tempDir, { recursive: true, force: true });
  tempDir = undefined;
});

function bytesToHex(bytes: Uint8Array): string {
  return Array.from(bytes).map((b) => b.toString(16).padStart(2, "0")).join("");
}

async function sha256Hex(bytes: Uint8Array): Promise<string> {
  return bytesToHex(new Uint8Array(await webcrypto.subtle.digest("SHA-256", bytes)));
}

async function generateKeyPair() {
  const { privateKey, publicKey } = (await webcrypto.subtle.generateKey(
    { name: "Ed25519" },
    true,
    ["sign", "verify"],
  )) as CryptoKeyPair;
  const publicKeyBytes = new Uint8Array(await webcrypto.subtle.exportKey("raw", publicKey));
  const signerKeyId = await sha256Hex(publicKeyBytes);
  return { privateKey, publicKeyBytes, signerKeyId };
}

// Mirrors `integrity::sign_package`'s exact envelope shape (see src-tauri/src/integrity.rs), so a
// file built here is indistinguishable in structure from one this application actually produced.
async function signPackage(manifest: unknown, key: Awaited<ReturnType<typeof generateKeyPair>>) {
  const canonicalBytes = Buffer.from(JSON.stringify(manifest), "utf8");
  const manifestSha256 = await sha256Hex(canonicalBytes);
  const signatureBytes = new Uint8Array(
    await webcrypto.subtle.sign("Ed25519", key.privateKey, canonicalBytes),
  );
  return {
    manifestCanonicalBase64: canonicalBytes.toString("base64"),
    manifestSha256,
    manifest,
    signature: {
      algorithm: "Ed25519",
      publicKeyHex: bytesToHex(key.publicKeyBytes),
      signerKeyId: key.signerKeyId,
      signatureHex: bytesToHex(signatureBytes),
      identityAssurance: "SELF_MANAGED_LOCAL_KEY_NOT_EXTERNALLY_ATTESTED",
    },
  };
}

function caseManifest(caseId: string, sequence: number, previousExportSha256: string | null) {
  return {
    schemaVersion: "onrecord-case-evidence-package-v2",
    generatedAt: "2026-09-28T00:00:00Z",
    case: { id: caseId, evidence: [] },
    verifiedCitationCount: 0,
    unverifiedCitationLabels: [],
    withheldCitations: [],
    previousExportSha256,
    ledgerSequence: sequence,
  };
}

function revocationListManifest(entries: unknown[]) {
  return { schemaVersion: "onrecord-signing-key-revocations-v1", entries };
}

async function setup() {
  tempDir = await mkdtemp(join(tmpdir(), "verify-cli-"));
  return tempDir;
}

async function writePackage(dir: string, name: string, signed: unknown): Promise<string> {
  const path = join(dir, name);
  await writeFile(path, JSON.stringify(signed, null, 2));
  return path;
}

describe("verify-evidence-package.mjs CLI, end to end", () => {
  it("passes a correctly linked consecutive pair", async () => {
    const dir = await setup();
    const key = await generateKeyPair();
    const first = await signPackage(caseManifest("case-a", 1, null), key);
    const second = await signPackage(caseManifest("case-a", 2, first.manifestSha256), key);
    const p1 = await writePackage(dir, "a1.json", first);
    const p2 = await writePackage(dir, "a2.json", second);
    const { stdout } = await run("node", [scriptPath, p1, p2]);
    expect(stdout).toContain("Chain: PASS");
  });

  it("exits non-zero when a link is broken", async () => {
    const dir = await setup();
    const key = await generateKeyPair();
    const first = await signPackage(caseManifest("case-a", 1, null), key);
    const second = await signPackage(caseManifest("case-a", 2, "wrong-hash"), key);
    const p1 = await writePackage(dir, "a1.json", first);
    const p2 = await writePackage(dir, "a2.json", second);
    await expect(run("node", [scriptPath, p1, p2])).rejects.toMatchObject({
      code: 1,
      stdout: expect.stringContaining("Chain: FAIL"),
    });
  });

  it("checks a research snapshot's sibling file against the signed digest", async () => {
    const dir = await setup();
    const key = await generateKeyPair();
    const bytes = new TextEncoder().encode("duckdb bytes");
    const fileName = "snap.duckdb";
    const signed = await signPackage(
      { schemaVersion: "onrecord-research-snapshot-v1", fileName, fileSha256: await sha256Hex(bytes), corpusStateDigest: "x" },
      key,
    );
    const manifestPath = await writePackage(dir, "snap.duckdb.manifest.json", signed);
    await writeFile(join(dir, fileName), bytes);
    const { stdout } = await run("node", [scriptPath, manifestPath]);
    expect(stdout).toContain("PASS  Sibling file snap.duckdb");

    await writeFile(join(dir, fileName), "tampered");
    await expect(run("node", [scriptPath, manifestPath])).rejects.toMatchObject({
      code: 1,
      stdout: expect.stringContaining("FAIL  Sibling file snap.duckdb"),
    });
  });

  it("does not treat two different cases each at sequence 1 as a fork", async () => {
    const dir = await setup();
    const key = await generateKeyPair();
    const a1 = await signPackage(caseManifest("case-a", 1, null), key);
    const b1 = await signPackage(caseManifest("case-b", 1, null), key);
    const pA = await writePackage(dir, "a1.json", a1);
    const pB = await writePackage(dir, "b1.json", b1);
    const { stdout } = await run("node", [scriptPath, pA, pB]);
    expect(stdout).toContain("Chain: PASS");
  });

  it("reports a gap between non-consecutive sequences as informational, not a failure", async () => {
    const dir = await setup();
    const key = await generateKeyPair();
    const first = await signPackage(caseManifest("case-a", 1, null), key);
    const third = await signPackage(caseManifest("case-a", 3, "hash-of-sequence-2-never-given"), key);
    const p1 = await writePackage(dir, "a1.json", first);
    const p3 = await writePackage(dir, "a3.json", third);
    const { stdout } = await run("node", [scriptPath, p1, p3]);
    expect(stdout).toContain("GAP");
    expect(stdout).toContain("Chain: PASS");
  });

  it("fails when the same sequence in the same case is given twice with different hashes (a fork)", async () => {
    const dir = await setup();
    const key = await generateKeyPair();
    const original = await signPackage(caseManifest("case-a", 2, "hash-a1"), key);
    const forked = await signPackage({ ...caseManifest("case-a", 2, "hash-a1"), generatedAt: "2026-09-29T00:00:01Z" }, key);
    const p1 = await writePackage(dir, "a2-original.json", original);
    const p2 = await writePackage(dir, "a2-forked.json", forked);
    await expect(run("node", [scriptPath, p1, p2])).rejects.toMatchObject({
      code: 1,
      stdout: expect.stringContaining("forked chain"),
    });
  });

  it("exits 0 for a package signed by a key retired for routine rotation", async () => {
    const dir = await setup();
    const key = await generateKeyPair();
    const retiringKey = await generateKeyPair();
    const pkg = await signPackage(caseManifest("case-a", 1, null), retiringKey);
    const revocations = await signPackage(
      revocationListManifest([
        {
          keyId: retiringKey.signerKeyId,
          revokedAt: "2026-09-29T00:00:00Z",
          reason: "Routine hygiene rotation.",
          status: "RETIRED",
        },
      ]),
      key,
    );
    const pkgPath = await writePackage(dir, "pkg.json", pkg);
    const revPath = await writePackage(dir, "revocations.json", revocations);
    const { stdout } = await run("node", [scriptPath, "--revocations", revPath, pkgPath]);
    expect(stdout).toContain("Overall: PASS");
    expect(stdout).toContain("Retired");
  });

  it("exits non-zero for a package signed by a key revoked as compromised", async () => {
    const dir = await setup();
    const key = await generateKeyPair();
    const compromisedKey = await generateKeyPair();
    const pkg = await signPackage(caseManifest("case-a", 1, null), compromisedKey);
    const revocations = await signPackage(
      revocationListManifest([
        {
          keyId: compromisedKey.signerKeyId,
          revokedAt: "2026-09-29T00:00:00Z",
          reason: "Suspected compromise.",
          status: "COMPROMISED",
        },
      ]),
      key,
    );
    const pkgPath = await writePackage(dir, "pkg.json", pkg);
    const revPath = await writePackage(dir, "revocations.json", revocations);
    await expect(
      run("node", [scriptPath, "--revocations", revPath, pkgPath]),
    ).rejects.toMatchObject({
      code: 1,
      stdout: expect.stringContaining("COMPROMISED"),
    });
  });

  it("exits non-zero when a key has both a RETIRED and a COMPROMISED entry, regardless of order", async () => {
    const dir = await setup();
    const key = await generateKeyPair();
    const escalatedKey = await generateKeyPair();
    const pkg = await signPackage(caseManifest("case-a", 1, null), escalatedKey);
    const revocations = await signPackage(
      revocationListManifest([
        {
          keyId: escalatedKey.signerKeyId,
          revokedAt: "2026-09-29T00:00:00Z",
          reason: "Routine hygiene rotation.",
          status: "RETIRED",
        },
        {
          keyId: escalatedKey.signerKeyId,
          revokedAt: "2026-09-30T00:00:00Z",
          reason: "Later found on a leaked backup.",
          status: "COMPROMISED",
        },
      ]),
      key,
    );
    const pkgPath = await writePackage(dir, "pkg.json", pkg);
    const revPath = await writePackage(dir, "revocations.json", revocations);
    await expect(
      run("node", [scriptPath, "--revocations", revPath, pkgPath]),
    ).rejects.toMatchObject({
      code: 1,
      stdout: expect.stringContaining("COMPROMISED"),
    });
  });

  it("rejects a revocations file whose unsigned manifest.entries was tampered with", async () => {
    const dir = await setup();
    const key = await generateKeyPair();
    const compromisedKey = await generateKeyPair();
    const pkg = await signPackage(caseManifest("case-a", 1, null), compromisedKey);
    const revocations = await signPackage(
      revocationListManifest([
        {
          keyId: compromisedKey.signerKeyId,
          revokedAt: "2026-09-29T00:00:00Z",
          reason: "Suspected compromise.",
          status: "COMPROMISED",
        },
      ]),
      key,
    );
    // Strip the compromised entry from the unsigned `manifest` field only, leaving the signed
    // canonical bytes untouched — exactly the attack `loadRevocations` must catch.
    (revocations.manifest as { entries: unknown[] }).entries = [];
    const pkgPath = await writePackage(dir, "pkg.json", pkg);
    const revPath = await writePackage(dir, "revocations.json", revocations);
    await expect(
      run("node", [scriptPath, "--revocations", revPath, pkgPath]),
    ).rejects.toMatchObject({ code: 1 });
  });

  it("rejects a correctly signed package that is not a revocation list", async () => {
    const dir = await setup();
    const key = await generateKeyPair();
    const notARevocationList = await signPackage(caseManifest("case-a", 1, null), key);
    const pkgPath = await writePackage(dir, "pkg.json", notARevocationList);
    await expect(
      run("node", [scriptPath, "--revocations", pkgPath, pkgPath]),
    ).rejects.toMatchObject({ code: 1 });
  });

  it("rejects an unverifiable (corrupted) revocations file rather than treating it as absent", async () => {
    const dir = await setup();
    const key = await generateKeyPair();
    const pkg = await signPackage(caseManifest("case-a", 1, null), key);
    const revocations = await signPackage(revocationListManifest([]), key);
    revocations.signature.signatureHex = "00".repeat(64);
    const pkgPath = await writePackage(dir, "pkg.json", pkg);
    const revPath = await writePackage(dir, "revocations.json", revocations);
    await expect(
      run("node", [scriptPath, "--revocations", revPath, pkgPath]),
    ).rejects.toMatchObject({ code: 1 });
  });

  it("still runs correctly when invoked through a symlinked path", async () => {
    const dir = await setup();
    const key = await generateKeyPair();
    const pkg = await signPackage(caseManifest("case-a", 1, null), key);
    const pkgPath = await writePackage(dir, "pkg.json", pkg);
    const linkPath = join(dir, "verify-link.mjs");
    await symlink(scriptPath, linkPath);
    const { stdout } = await run("node", [linkPath, pkgPath]);
    expect(stdout).toContain("Overall: PASS");
  });
});

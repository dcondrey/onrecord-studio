import { describe, expect, it } from "vitest";
import { readFile } from "node:fs/promises";
import { join } from "node:path";
import { createContext, runInContext } from "node:vm";
import { webcrypto } from "node:crypto";

// `scripts/verify-evidence-package.html` ships to recipients who have a browser and nothing else,
// so its verify function must agree with the Rust signer on both committed signed-package schemas
// and must reject a single flipped byte. The page's script is run here in its own context with
// Node's WebCrypto standing in for the browser's: the page is DOM-optional by construction (the
// listener wiring is guarded) precisely so this contract is testable without a renderer.
const pagePath = join(import.meta.dirname, "../../scripts/verify-evidence-package.html");
const fixtureDir = join(import.meta.dirname, "../../src-tauri/tests/fixtures");

type VerifyResult = {
  passed: boolean;
  checks: { passed: boolean; label: string }[];
  schemaVersion: string | null;
  signerKeyId: string | null;
  problem: string | null;
};

async function loadPageVerifier(): Promise<(text: string) => Promise<VerifyResult>> {
  const html = await readFile(pagePath, "utf8");
  const script = html.match(/<script>([\s\S]*?)<\/script>/)?.[1];
  if (!script) throw new Error("The verifier page carries no inline script.");
  if (/\bsrc=|\bfetch\(|XMLHttpRequest/.test(html)) {
    throw new Error("The verifier page must stay self-contained and network-free.");
  }
  const context = createContext({ crypto: webcrypto, atob, TextDecoder });
  runInContext(script, context);
  return (context as { onrecordPackageVerifier: { verifyPackage: (text: string) => Promise<VerifyResult> } })
    .onrecordPackageVerifier.verifyPackage;
}

const fixtures = [
  ["evidence-package-v3-fixture.json", "onrecord-evidence-package-v3"],
  ["request-filing-record-v1-fixture.json", "onrecord-request-filing-record-v1"],
] as const;

describe("static browser package verifier", () => {
  it.each(fixtures)("verifies %s produced by the Rust signer", async (file, schema) => {
    const verifyPackage = await loadPageVerifier();
    const result = await verifyPackage(await readFile(join(fixtureDir, file), "utf8"));
    expect(result.checks.map((check) => check.passed)).toEqual([true, true, true]);
    expect(result.schemaVersion).toBe(schema);
    expect(result.passed).toBe(true);
  });

  it.each(fixtures)("rejects %s after a one-byte mutation of the signed bytes", async (file) => {
    const verifyPackage = await loadPageVerifier();
    const original = JSON.parse(await readFile(join(fixtureDir, file), "utf8"));
    const bytes = Buffer.from(original.manifestCanonicalBase64, "base64");
    bytes[bytes.length - 2] ^= 0x01;
    const mutated = { ...original, manifestCanonicalBase64: bytes.toString("base64") };
    const result = await verifyPackage(JSON.stringify(mutated));
    expect(result.passed).toBe(false);
    expect(result.checks.filter((check) => check.passed)).toHaveLength(0);
  });

  it("rejects a package whose signer key id does not hash from the key that verified", async () => {
    const verifyPackage = await loadPageVerifier();
    const original = JSON.parse(await readFile(join(fixtureDir, fixtures[0][0]), "utf8"));
    original.signature.signerKeyId = "0".repeat(64);
    const result = await verifyPackage(JSON.stringify(original));
    expect(result.signerKeyId).toBeNull();
    expect(result.passed).toBe(false);
  });
});

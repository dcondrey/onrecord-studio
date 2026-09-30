import { describe, expect, it, afterEach } from "vitest";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { mkdtemp, readFile, writeFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

// Locks the cross-language contract from the other side: `scripts/verify-evidence-package.mjs`
// must accept the exact bytes `src-tauri/src/storage/casework.rs`'s
// `case_evidence_package_bytes_match_the_committed_cross_language_fixture` test locks on the Rust
// side, and must reject a tampered copy. Neither language's test alone proves the two agree.
const run = promisify(execFile);
const fixturePath = join(
  import.meta.dirname,
  "../../src-tauri/tests/fixtures/chain-of-custody-fixture.json",
);
const scriptPath = join(import.meta.dirname, "../../scripts/verify-evidence-package.mjs");

let tempDir: string | undefined;
afterEach(async () => {
  if (tempDir) await rm(tempDir, { recursive: true, force: true });
  tempDir = undefined;
});

describe("standalone chain-of-custody verifier", () => {
  it("passes the committed fixture produced by the Rust signer", async () => {
    const { stdout } = await run("node", [scriptPath, fixturePath]);
    expect(stdout).toContain("Overall: PASS");
  });

  it("fails on a copy whose manifest field was edited after signing", async () => {
    tempDir = await mkdtemp(join(tmpdir(), "custody-verify-"));
    const original = JSON.parse(await readFile(fixturePath, "utf8"));
    original.manifest.verifiedCitationCount = 999;
    const tamperedPath = join(tempDir, "tampered.json");
    await writeFile(tamperedPath, JSON.stringify(original, null, 2));
    await expect(run("node", [scriptPath, tamperedPath])).rejects.toMatchObject({
      code: 1,
      stdout: expect.stringContaining("Overall: FAIL"),
    });
  });
});

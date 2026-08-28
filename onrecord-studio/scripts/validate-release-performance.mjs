import { spawnSync } from 'node:child_process';
import { performance } from 'node:perf_hooks';

const targetDirectory = process.env.ONRECORD_PERFORMANCE_TARGET_DIR
  ?? '/tmp/onrecord-release-performance-target';
const cases = [
  {
    label: 'million rows, large text, index rebuild, and concurrent reads',
    filter: 'storage::duckdb::tests::release_scale_harness_covers_million_rows_large_text_indexes_and_concurrent_reads',
    maximumSeconds: 60,
  },
  {
    label: '100,000-link structured discovery bound',
    filter: 'ingestion::queue::tests::release_stress_harness_bounds_high_link_discovery',
    maximumSeconds: 60,
  },
  {
    label: 'parser-memory backpressure',
    filter: 'ingestion::queue::tests::release_stress_harness_applies_parser_memory_backpressure',
    maximumSeconds: 30,
  },
  {
    label: 'durable restart recovery',
    filter: 'storage::sqlite::tests::interrupted_frontier_is_recovered_into_a_new_run',
    maximumSeconds: 30,
  },
];

const commonOptions = {
  cwd: new URL('..', import.meta.url),
  encoding: 'utf8',
  env: { ...process.env, CARGO_TARGET_DIR: targetDirectory },
};
const warmup = spawnSync(
  'cargo',
  ['test', '--manifest-path', 'src-tauri/Cargo.toml', '--offline', '--no-run'],
  commonOptions,
);
if (warmup.status !== 0) {
  process.stderr.write(warmup.stdout);
  process.stderr.write(warmup.stderr);
  throw new Error('Release performance harness failed to compile');
}

for (const releaseCase of cases) {
  const startedAt = performance.now();
  const result = spawnSync(
    'cargo',
    [
      'test',
      '--manifest-path',
      'src-tauri/Cargo.toml',
      '--offline',
      releaseCase.filter,
      '--',
      '--exact',
    ],
    commonOptions,
  );
  const elapsedSeconds = (performance.now() - startedAt) / 1_000;
  if (result.status !== 0) {
    process.stderr.write(result.stdout);
    process.stderr.write(result.stderr);
    throw new Error(`Release performance case failed: ${releaseCase.label}`);
  }
  if (elapsedSeconds > releaseCase.maximumSeconds) {
    throw new Error(
      `${releaseCase.label} took ${elapsedSeconds.toFixed(2)}s; budget is ${releaseCase.maximumSeconds}s`,
    );
  }
  process.stdout.write(`PASS ${releaseCase.label}: ${elapsedSeconds.toFixed(2)}s\n`);
}

# Automatic discovery preserves source ownership

`QueueManager::discover_and_start_public_source_automatically` resolves the URL
before admitting the source. Under the same provisioning lock used by pause and
removal, it checks both the resolved portal key and canonical publication URL,
including withdrawn registrations. Existing registrations are rejected before
either registration or connector-recipe writes. This also prevents a changed
platform classification from bypassing the saved source's lifecycle.

New sources retain the submitted alias and bounded crawl recipe. Explicit
operator starts keep their existing ability to resume a paused source. An
existing active source continues through its saved refresh schedule; automatic
discovery does not overwrite its collection instructions.

Three offline tests passed in the normal 451-test native suite:

- `automatic_discovery_preserves_resolved_source_lifecycle_and_recipe` checks
  paused, removed and active registrations, a changed source key at the same
  canonical URL, untouched recipes and absence of spurious sync runs.
- `automatic_discovery_rechecks_identity_after_a_concurrent_pause` queues an
  actual pause before automatic provisioning under the shared lifecycle lock.
- `automatic_discovery_accepts_new_sources_and_operator_starts_can_resume_them`
  checks new-source admission, alias recipe lookup and explicit operator resume.

The first two guards were mutation-proven in an isolated source copy by disabling
only the `AutomaticDiscovery` identity-check predicate in `start_task_pool`.
Each exact test failed with exit 101: the first observed an unexpected successful
start, and the second observed reactivation after pause. The ordinary workspace
source was never mutated. Payload workers were prevented from accessing the
network in these tests. The separate public SDHC acquisition check exercises real
download and extraction behavior.

The positive start/resume test subsequently exposed an intermittent production
deadlock: stopping held the provisioning lock while joining a worker whose idle
retirement was waiting for that same lock. Retirement now observes cancellation
while waiting for the lock, after the current task's writes finish. Parser work
is not aborted. Each positive-test start/pause has a bounded diagnostic, and a
separate regression holds the joiner's lock while requiring worker retirement
after cancellation. Restoring the former unconditional wait produces the exact
three-second timeout; see the [isolated negative-control trace](collection-cancellation-negative-control-2026-09-27.txt).

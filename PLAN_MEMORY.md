# PLAN_MEMORY.md — Memory and Compatibility Budget

## Scope
Own measurement/reporting and memory/layout test design. Read existing size assertions, `AllocationHeader`, frozen descriptor representations, `CageConfig`, allocator statistics, retained owner definitions, performance examples. Production type or allocator layout changes owned by relevant allocator/collections workstream.

## V2.4 baseline and targets
Record size_of/align_of for `CageAllocation<T>`, `Option<CageAllocation<T>>`, `CompactBox<T>`, `CompactVec<T>`, `CompactVecDeque<T>`, AllocationHeader and frozen descriptors. On exact equivalent 16 scenarios record live/retained bytes, metadata per object, allocated/cached capacity, cage high-water, unallocated/free extents, fragmentation, virtual reservation, resident/committed pages, peak RSS, post-quiescence idle RSS, worker-thread local caches.

Default integrated V2.5 gates: ≤+2% retained, ≤+5% peak RSS, approximately baseline idle, no significant fragmentation or per-thread scaling regression. Compare *absolute bytes* and ratios. For small baselines avoid meaningless percentage alarms and document statistical noise. Enforce bounded TLS caches and reclamation after thread exit.

## Architecture-change review
For every contract modification provide byte-level delta, workload distribution, peak/idle effects, fragmentation, source API/ABI/serialized/frozen-format compatibility and migration. Preserve 4-byte owners when feasible; any layout or budget exception requires explicit orchestrator/user approval and measured CPU/throughput advantage. Do not equate reserved virtual cage with resident memory or add telemetry overhead to timing binaries.

## Tests / deliverable
Stress tiny/large/mixed allocations, high thread churn, remote frees, long-lived objects, fragmentation, idle/quiescence and steady state. Produce `PROFILE_V2_5_MEMORY.md` with per-scenario baseline versus integrated totals, exceptions and mitigation; no unilateral runtime-layout code edits.

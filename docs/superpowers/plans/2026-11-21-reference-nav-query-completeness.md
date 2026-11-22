# Reference navmesh query completeness

> **For agentic workers:** Preserve the runtime query and bake formats. Use the existing offline reference artifacts to verify the targeted behavior.

**Goal:** Report proven disconnection without spending a node pool on an unreachable reference goal, and recover the observed reachable reference route with a bounded search.

**Evidence:** The original 33 requests reproduce 11 Complete, 11 Partial, and 11 OutOfNodes. Every Partial and ten OutOfNodes requests have no eligible standing goal in the start component. All six loaded static graphs have reciprocal links. The remaining request reaches the same selected goal with a reverse search using 6,933 of the existing 16,384 nodes and 372 forward-valid polygon references.

- [x] Add regressions for provable disconnection and for one reference reverse retry at the existing node limit. Keep synthetic fixtures independent of the retail inputs.
- [x] For a reference query, report Disconnected only when the full selected goal set is available, no eligible candidate is in the source component, and loaded edges cannot cross component labels. Keep projected endpoint observations and omit a speculative corridor.
- [x] For a same-component reference query that hits the node pool before reaching its selected goal, try that exact endpoint in reverse once. Require a complete, untruncated reverse result and validate every resulting forward link before publication. Preserve the original result if the retry does not complete.
- [x] Verify exact artifact digests across the old and new 33-request replay. Preserve the original 11 completed route bodies, recover the one connected query, and report the other 21 as Disconnected. Keep authored objective interaction targets and missing geometry separate.

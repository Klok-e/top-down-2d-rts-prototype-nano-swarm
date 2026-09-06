---
name: testing-on-the-toilet
description: >-
  Write and review tests that catch meaningful behavior failures. Use when
  assessing test coverage, redundant or brittle tests, test doubles, assertions,
  or nondeterministic tests. Repository setup and test commands belong in project docs.
---

# Testing on the Toilet

Apply these decision rules when writing or reviewing tests. A useful test catches an identifiable behavior failure and remains valid through refactors that preserve its contract.

The guidance draws on Google's Testing on the Toilet series. Read the [episode index](references/episodes.md) when you need a source or a topic compressed here.

## 1. Change-detector gate

Identify the contract and a concrete fault the test would catch. Contracts include observable results, side effects, and internal invariants such as a persisted encoding or resource conservation. Prefer public behavior; testing a private function is justified when the invariant itself matters.

A change-detector asserts implementation choices that can change without violating the contract, such as an incidental collaborator call order. Replace those assertions with the relevant outcome. Verify interactions when the interaction is the contract, such as sending a notification once.

Before deleting a test, establish that it duplicates retained coverage or protects no meaningful contract. Inspect callers and nearby coverage when its purpose is unclear; unresolved understanding is a reason to report uncertainty, not evidence for deletion. A tautology provides no behavioral protection, but a useful scenario may warrant replacing its assertion rather than discarding it.

Apply recommendations within the user's authorized scope. A testability concern can justify a proposed production refactor; it does not itself authorize one. Resolve instruction conflicts using the applicable instruction priority and existing user decisions. Ask only when an unresolved choice materially affects the work and cannot be settled from available evidence.

## 2. Pick the cheapest layer that still has fidelity

Choose the least costly test that can expose the identified fault with adequate fidelity. Use pure logic tests for calculations, real in-process collaborators for their integration, and broader flows when wiring or cross-system behavior is the risk. For UI wiring, exercise the control through the application's supported interaction seam; calling its handler alone does not prove the wiring.

Compare speed, maintainability, resource utilization, reliability, and fidelity when layers overlap. A lower test count or higher coverage percentage is not evidence of better protection. Preserve materially different failure cases without multiplying scenarios that catch the same fault.

## 3. Choose the double

Prefer a real dependency when it is fast, controllable, and isolated. Otherwise choose a fake for working behavior, a stub for a supplied response, or a mock when an interaction needs verification. Check a shared fake against the real dependency's relevant contract.

Prefer an owned boundary over mocking third-party internals. If constructing that boundary requires out-of-scope production changes, report the tradeoff. Long mock scripts suggest a brittle seam; consider a simpler fixture or different test layer before proposing architectural changes.

## 4. Author the test (DAMP)

Keep cause and effect visible. DAMP means descriptive and meaningful phrases: helpers may hide irrelevant setup, while scenario-defining values and the asserted outcome remain easy to inspect.

- Name the behavior and scenario using project conventions. Group assertions that establish one coherent outcome; separate unrelated behaviors.
- Derive expected results independently of production logic. Prefer literal examples for simple cases; use independent oracles or invariant checks when that better expresses the contract. Test helper logic when its complexity or reuse creates a material risk of false results.
- Assert relevant fields with informative failures. Whole-value equality is appropriate when the whole value is the contract. Use distinguishable inputs where defaults could conceal a fault; retain zero and empty values when those boundaries are the scenario.
- Use explicit tolerances for approximate numerical results. Exact equality is appropriate for exactly representable results or values required to pass through unchanged.
- For UI tests, choose stable locators appropriate to the contract: test IDs, semantic roles, or accessible names. Assert wording when wording is itself the requirement.

## 5. Hermetic and deterministic

Control the source of nondeterminism. Advance simulated time for time-dependent logic and synchronize on explicit completion with a bounded timeout or simulated-step limit instead of sleeping. Use isolated temporary storage when real filesystem behavior matters, and controlled dependencies for external services. Avoid shared state or live infrastructure for small logic tests.

Exercise rare error paths through controlled inputs or dependencies rather than hoping a live failure occurs. Use the repository's supported test seams and execution constraints.

## 6. Prove the test can fail

For new or changed behavioral coverage, reproduce the original bug or introduce a representative production fault and confirm that the test fails for the expected behavioral reason. An inverted assertion, compile failure, or unrelated setup failure does not establish this proof.

Before deleting or consolidating duplicate coverage, identify the retained test and demonstrate that it detects the relevant fault. Keep that fault active through the test refactor so lost protection is observable. Restore temporary production changes without overwriting others' work, then confirm the final tests pass using the repository's required checks.

During read-only reviews, do not edit tests or temporarily mutate production. Describe the concrete fault the coverage should catch and distinguish that reasoning from executed failure proof. If execution is unavailable, report the missing evidence without claiming verification occurred.

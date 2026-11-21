---
name: implement
description: "Implement a piece of work based on a spec or set of tickets."
---

Implement the work described by the user in the spec or tickets.

If the user passes a ticket reference, fetch it from the issue tracker and state its title before starting. If the reference is ambiguous, ask.

Record the starting commit before editing. If the user has agreed on a public interface to test, build one behavior at a time through that interface. Confirm that the test fails before implementing the behavior and passes afterward.

Run typechecking regularly, single test files regularly, and the full test suite once at the end.

Commit the implementation to the current branch. Run `code-review` against the recorded starting commit with the originating spec or ticket. Fix actionable findings and commit the fixes.

## Summary

<!-- What outcome changes, and why? -->

## Specification and contracts

- Spec invariants / SI / RF affected:
- Test contract IDs affected:
- Protected effect: <!-- What must never happen when the negative case is exercised? -->

## Positive evidence

<!-- Name the registered tests and say what valid behavior they prove. -->

- `test_name` —

## Negative evidence

<!-- Name the registered tests, the invalid condition, and the protected effect asserted absent. -->

- `test_name` —

## Mutation evidence

<!-- Name the stable function/file filter exercised by `scripts/ci mutation`, or explain why mutation testing cannot apply. -->

- Target:
- Result / rationale:

## Verification

- [ ] Every new or renamed Rust test is registered in `tests/contracts.tsv`.
- [ ] Every new contract has at least one positive and one negative test.
- [ ] Negative tests assert that the protected effect did not occur.
- [ ] Enforcement changes exercise a stable targeted mutation lane, or the exception is justified above.
- [ ] `./scripts/ci required` passes locally.

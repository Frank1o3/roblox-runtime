## Summary

Describe the subsystem and the problem this change addresses.

## Behavior and evidence

- Observed caller, ABI contract, or runtime behavior:
- Previous behavior:
- New behavior:

## Verification

- [ ] cargo fmt --all -- --check
- [ ] cargo check --workspace
- [ ] cargo test --workspace
- [ ] Manual runtime checks are described accurately.
- [ ] Documentation and provenance notes are updated where needed.

Commands run and results:

## Safety and compatibility

- [ ] Unsafe code includes a documented safety invariant.
- [ ] JNI/native pointer lifetimes and thread ownership were reviewed, if relevant.
- [ ] Capture still opens only while recording, if audio behavior changed.
- [ ] No cookies, tokens, credentials, or private user data were added.
- [ ] Third-party copyright and license notices are preserved.

## Checklist

- [ ] I reviewed the Code of Conduct and contributing guide.
- [ ] I explained unsupported or inferred behavior instead of claiming it is verified.
- [ ] This pull request is focused and includes tests for changed behavior.

# Pinned engine-import entry oracle

Every `.ts` file here is a verbatim `git show 744d14af:src/<path>` copy from Lair, named after its path with `/`
replaced by `-`. Because Lair names also contain `-`, the mapping is listed explicitly:

| file | Lair path |
|---|---|
| backend-accounts.ts | src/backend/accounts.ts |
| backend-codex-home.ts | src/backend/codex-home.ts |
| backend-engine-import.ts | src/backend/engine-import.ts |
| backend-engine-import-bound.ts | src/backend/engine-import/bound.ts |
| backend-engine-import-claude.ts | src/backend/engine-import/claude.ts |
| backend-engine-import-codex.ts | src/backend/engine-import/codex.ts |
| backend-engine-import-qwen.ts | src/backend/engine-import/qwen.ts |
| backend-engine-import-tail.ts | src/backend/engine-import/tail.ts |
| backend-migrate-work-run.ts | src/backend/migrate/work-run.ts |
| backend-requests-engine-import.ts | src/backend/requests/engine-import.ts |
| shared-contracts.ts | src/shared/contracts.ts |
| shared-engine-import.ts | src/shared/engine-import.ts |

`grok.ts`, `opencode.ts` and `plan.ts` at the same pin are in `bench/l3/grok/reference/`. Line numbers in the L3-8 spec
cite these copies, which equal the pinned paths.

Lair `src/backend/engine-import.ts`, `src/backend/engine-import/` and `src/backend/accounts.ts` are unchanged from
744d14af to Lair HEAD on 2026-11-17; `src/backend/codex-home.ts` changed after the pin and L3-8 follows the pin.

SHA-256:

| file | sha256 |
|---|---|
| backend-accounts.ts | `2f6e8a90948e2d1581e284b8bb55b1389b23e9eead8509327975d88fdd35a051` |
| backend-codex-home.ts | `6eec39476f614c0f77aabfad3b75419d287f55d1d5b4815f8f71783925e47706` |
| backend-engine-import-bound.ts | `b2566f1d0e33ac0bcdbf95fbd99c33269d1396fd78d8afdee9967872767b9baa` |
| backend-engine-import-claude.ts | `562704a93d40d16f0fc0327e65c704a6db0090d476943484d4ee8030f86b8945` |
| backend-engine-import-codex.ts | `19e07785fa085e60c78937730aef76dd00d1d0a99b80ad08008b4fb4008dfdb8` |
| backend-engine-import-qwen.ts | `3e2b034f5b0ad6dc46136ace3faa793c4f1025e7db2111bd1373fb408f9dd280` |
| backend-engine-import-tail.ts | `750399578ca53bcf6ccf0b2b03863de08d0f16a3580c60f40d9ef8c216c3c14e` |
| backend-engine-import.ts | `f234c748f1241021ce0f75a3a823dbe7931c4a954470119bb5a490a36f5946f5` |
| backend-migrate-work-run.ts | `bf2ae50e4aa63e8e6984d42577d9860de8f8707a183ac6e778ff5bf0f25d5710` |
| backend-requests-engine-import.ts | `861c83acbc9209e04364b3e50aec349d512103388c2ee79defc41bf2dd52b9cd` |
| shared-contracts.ts | `042e349e5c3024e731984852d5765fbe36584ba4f584799d713fc75553217af9` |
| shared-engine-import.ts | `89cd7c0acb6cee4fb2534f3a9a7fffa2032de5e3eba7a9231bfa9e8851a1d583` |

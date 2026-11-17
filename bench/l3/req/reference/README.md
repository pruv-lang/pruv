# Pinned reference for L3-9 (engine-import request handlers)

Verbatim copies from Lair commit `17001ab3` (`git show 17001ab3:<path>`). Do not edit.

| File | Lair path | SHA-256 |
|---|---|---|
| `backend-codex-home.ts` | `src/backend/codex-home.ts` | `5e45b844488aa1c88ccc8419fb3f1457488bcbaea952167da6263be1bd4cf855` |
| `backend-inputs-engine-import.ts` | `src/backend/inputs/engine-import.ts` | `0fee23d9f8a9af059fe29da8966bacbb40e60a80709439295da5536b301597b5` |
| `backend-migrate-work-run.ts` | `src/backend/migrate/work-run.ts` | `a448618b3c79d67bc1e30a12304a79b4f917e672447b48c5aadc9af6b3d0f9eb` |
| `backend-requests-engine-import.ts` | `src/backend/requests/engine-import.ts` | `7beac9168b337dc2e0372a4cd98c02895e013f09149a26f1b183deef52869093` |
| `backend-requests-types.ts` | `src/backend/requests/types.ts` | `fa686ae6c0c96362e9c7d0e34e7c3365d0900713f6f7f7b7e482c78bd2dc8af5` |
| `logic-records.ts` | `src/logic/records.ts` | `e5e4e250ff433a841f6ca8581deeb3a6df967c9820c94909801697362de90b35` |
| `shared-contracts.ts` | `src/shared/contracts.ts` | `48c75105a339e6f5e41a306087a53f07e4fe59be06d8800992c193efdaca3a59` |
| `shared-engine-import.ts` | `src/shared/engine-import.ts` | `89cd7c0acb6cee4fb2534f3a9a7fffa2032de5e3eba7a9231bfa9e8851a1d583` |
| `shared-engine-thread.ts` | `src/shared/engine-thread.ts` | `f812d8f7e281d72e767260cafffb63767e890736aca268893514800ecb59065a` |

The engine-import functions this handler calls (importSources, findSource, listExternal, readExternal, sessionThreads, excludedFromBound, accountHome) are pinned in `bench/l3/entry/reference/` (Lair `744d14af`) and are byte-identical at `17001ab3`. Between the two commits `backend/requests/engine-import.ts` changed only the codex-unsupported message text (line 60); `backend/codex-home.ts`, `backend/migrate/work-run.ts` and `shared/contracts.ts` changed and are copied here at `17001ab3`.

# Pinned reference for L3-10 (request budgets)

Verbatim copies from Lair commit `17001ab3` (`git show 17001ab3:<path>`). Do not edit.

| File | Lair path | SHA-256 |
|---|---|---|
| `backend-budget-wiring.ts` | `src/backend/budget-wiring.ts` | `fd1405240c8d0ad144fe02dd25b843adab930e9a421373f99c7f75a8dba2d565` |
| `backend-request-budget.ts` | `src/backend/request-budget.ts` | `ca47bf5fdd077a15120857d10781fb18dbdfe8f082f39a4e11ab10d0fdc4e471` |
| `shared-engine-text.ts` | `src/shared/engine-text.ts` | `062ecec57056ad32374cb860ba02d22abb6c20c601757e5a2e9e12b8dc943284` |
| `shared-i18n-core.ts` | `src/shared/i18n/core.ts` | `4a3ca7186aa24736b9b23e7d59488e3da4d24d86f5a9c13962cc0dc87690afe4` |
| `shared-reply-blocks.ts` | `src/shared/reply-blocks.ts` | `f5c4a973e301a50d12503c1570f0fbfb150c1b49bc54c35e96a7b9d94cd40b01` |
| `shared-request-budget.ts` | `src/shared/request-budget.ts` | `7620f6c3fb9906e6df0f8830957c4d8782f71974b38e25545048bd706a43b6df` |
| `shared-schedule-lists.ts` | `src/shared/schedule-lists.ts` | `de85d35e600324f023078349a4d22507c81be032dbe604fe00d63745eb14dfd8` |
| `shared-sub-session-types.ts` | `src/shared/sub-session-types.ts` | `4c926e4c479af8033f3d6c20dfd101487f13b3a5ce38a66f5f0a188b10723ed1` |

`backend/request-budget.ts` imports `WireText` from `shared/i18n/wire` as a type only, so that file is not pinned. `backend/budget-wiring.ts` is pinned as the wiring reference (which deps the app passes); it is not ported.

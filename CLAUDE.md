# Lyftar

Design choices are the owner's; Claude may suggest. Talk them through in chat, one question at a time, with no popups
or pre-made answers. `DECISIONS.md` records what was chosen.

## Git

- Branch: `main`. Author: `Tankun Sriket <tankunsriket63741ecff056.invalid>` (repo-local config, already set).
- Every commit ends with the trailer `Co-Authored-By: design claude <noreply@anthropic.com>`.
- Commit subjects are `kind.folder.narrowing: sentence`, as in Soclets/CyborgPL. Kinds: `decision`, `design`,
  `feature`, `fix`, `test`, `house`. The second word is the folder, or the topic for `decision`, `design` and `house`.
  The third word is optional, and there is never a fourth.
- A `decision.*` commit credits who chose it and gives its number, e.g. `(owner: "yes"; D2)`.

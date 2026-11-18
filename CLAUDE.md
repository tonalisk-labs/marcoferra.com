# marcoferra.com

Marco Ferra's site, marcoferra.com, and its booking API.

- `mise run check` before every push, and keep it green.
- Commits follow Conventional Commits, and each describes the code it changes.
- Comments say only what the code can't: why, an invariant, an edge case, an outside constraint.
- Shared rules come from `@tonalisk-labs/kit`. Fix kit when it falls short, never work around it here.
- The site's copy is Marco's. Change it only where a fact changes, and keep it to plain
  punctuation: no dashes, no semicolons.
- Nothing public names Marco's current employer. The E2E guard reads the names from
  `PRIVATE_NAMES`. His CV is on request only, so neither it nor his photo is in the tree.
- `server/openapi.yaml` is the API's contract. Change the server, then the contract:
  `mise run contract` checks they agree, and `mise run generate` makes the site's client from it.

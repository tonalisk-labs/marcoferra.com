import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';

import { hashes, LOCK } from './cards.ts';

await test('every committed social card was drawn from its current text, template and fonts', () => {
  const drawn: unknown = JSON.parse(readFileSync(LOCK, 'utf8'));
  assert.deepEqual(drawn, hashes(), 'a card is stale: mise run cards');
});

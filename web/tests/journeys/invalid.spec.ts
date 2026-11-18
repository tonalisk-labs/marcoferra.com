// Invalid input: no Idempotency-Key, a body not JSON, a form sent too soon, no such type and
// every member wrong are each refused, naming what; so are the free times of no such type.
import { apiTest, expect } from '@tonalisk-labs/kit/testing';

import site from '../../../marcoferra.config.json' with { type: 'json' };
import {
  availability,
  availabilityReply,
  book,
  callerFor,
  merge,
  raw,
  readyBooking,
  visitor,
} from '../support/api.ts';

// Covers: limits.min_fill_seconds, limits.name_max_chars

const pointerOf = (member: unknown): string =>
  typeof member === 'object' && member !== null && 'pointer' in member
    ? String(member.pointer)
    : '';

const pointersOf = (data: unknown): string[] =>
  typeof data === 'object' && data !== null && 'errors' in data && Array.isArray(data.errors)
    ? data.errors.map(pointerOf)
    : [];

apiTest(
  'a request missing its key, or not JSON, is refused before anything is read',
  { tag: '@api' },
  async ({ stack }) => {
    const request = await readyBooking(callerFor(stack), 'unkeyed');
    const unkeyed = await raw(callerFor(stack), '/v1/bookings', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify(request),
    });
    expect(unkeyed.status).toBe(400);
    expect(unkeyed.headers.get('content-type')).toBe('application/problem+json');
    const problem: unknown = await unkeyed.json();
    expect(pointersOf(problem)).toEqual(['header:Idempotency-Key']);
    const text = await raw(callerFor(stack), '/v1/bookings', {
      method: 'POST',
      headers: { 'content-type': 'text/plain', 'idempotency-key': crypto.randomUUID() },
      body: 'name=Spam',
    });
    expect(text.status).toBe(415);
    const untyped = await availabilityReply(callerFor(stack), 'no-such-type');
    expect(untyped.status).toBe(400);
    expect(pointersOf(untyped.data)).toEqual(['query:type']);
  },
);

apiTest(
  'every invalid member is named, and a form sent at once is refused',
  { tag: '@api' },
  async ({ stack }) => {
    const caller = callerFor(stack);
    const request = await readyBooking(caller, 'invalid');
    const wrong = await book(caller, {
      type: request.type,
      start: '2020-01-06T10:00:00Z',
      name: 'x'.repeat(site.limits.name_max_chars + 1),
      email: 'not an address',
      website: 'https://spam.example',
      timeZone: 'Mars/Olympus_Mons',
      formToken: `${request.formToken}x`,
    });
    expect(wrong.status).toBe(400);
    const named = ['/email', '/formToken', '/name', '/start', '/timeZone', '/website'];
    expect(pointersOf(wrong.data).toSorted()).toEqual(named);
    const unknown = await book(caller, merge(request, { type: 'no-such-type' }));
    expect(unknown.status).toBe(400);
    expect(pointersOf(unknown.data)).toEqual(['/type']);
    const hasty = callerFor(stack);
    const { formToken } = await availability(hasty);
    const entry = merge(merge(request, visitor('hasty')), { formToken });
    const soon = await book(hasty, entry);
    expect(soon.status).toBe(400);
    expect(pointersOf(soon.data)).toEqual(['/formToken']);
  },
);

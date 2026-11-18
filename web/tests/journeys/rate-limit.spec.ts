// The request budget: an address over its hourly booking budget is refused with 429 and a
// Retry-After saying when to come back.
import { apiTest, expect } from '@tonalisk-labs/kit/testing';

import site from '../../../marcoferra.config.json' with { type: 'json' };
import { type Caller, callerFor, raw } from '../support/api.ts';

// Covers: rate_limits.bookings_per_ip_per_hour

const BUDGET = site.rate_limits.bookings_per_ip_per_hour;

/** Sends `count` empty booking requests from one caller, answering the last one's response. */
const attempts = async (caller: Caller, count: number): Promise<Response> => {
  const response = await raw(caller, '/v1/bookings', {
    method: 'POST',
    headers: { 'content-type': 'application/json', 'idempotency-key': crypto.randomUUID() },
    body: '{}',
  });
  return count <= 1 ? response : attempts(caller, count - 1);
};

apiTest(
  'one address past its hourly budget is refused with Retry-After',
  { tag: '@api' },
  async ({ stack }) => {
    const caller = callerFor(stack);
    const within = await attempts(caller, BUDGET);
    expect(within.status).toBe(400);
    const past = await attempts(caller, 1);
    expect(past.status).toBe(429);
    expect(past.headers.get('content-type')).toBe('application/problem+json');
    const wait = Number(past.headers.get('retry-after'));
    expect(wait).toBeGreaterThan(0);
    expect(wait).toBeLessThanOrEqual(3600);
    const other = await attempts(callerFor(stack), 1);
    expect(other.status, 'another address keeps its own budget').toBe(400);
  },
);

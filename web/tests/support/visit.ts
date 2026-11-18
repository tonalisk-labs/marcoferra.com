import { expect, type Page } from '@playwright/test';
import { type Coordinates, mailbox, type Summary } from '@tonalisk-labs/kit/testing';

import type { EventType } from '../../src/booking/event-types.ts';
import { claimStart, pastFillTime, services, visitor } from './api.ts';
import { MEETING } from './meeting.ts';
import { chooseStart, fillDetails, send, zoneOf } from './ui.ts';

export interface Visit {
  readonly start: string;
  readonly zone: string;
  readonly guest: { readonly name: string; readonly email: string };
}

/** On the type's page, opened unless the browser is there: a claimed time and a fresh visitor. */
export const fillVisit = async (
  page: Page,
  stack: Coordinates,
  { label, kind = MEETING }: { readonly label: string; readonly kind?: EventType },
): Promise<Visit> => {
  const start = await claimStart(stack, kind);
  const guest = visitor(label);
  const path = `/book/${kind.slug}/`;
  if (new URL(page.url()).pathname !== path) {
    await page.goto(path);
  }
  const zone = await zoneOf(page);
  await chooseStart(page, start, zone);
  await fillDetails(page, guest.name, guest.email);
  return { start, zone, guest };
};

/** Sends the form, and answers the invitation once the page has said to check the email. */
export const sendForInvitation = async (
  page: Page,
  stack: Coordinates,
  email: string,
): Promise<Summary> => {
  const arriving = mailbox(services(stack).mailpit).waitFor(email);
  await send(page, pastFillTime);
  await expect(page.getByRole('heading', { name: 'Check your email' })).toBeFocused();
  return arriving;
};

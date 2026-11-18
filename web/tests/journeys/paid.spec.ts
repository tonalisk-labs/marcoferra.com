// A paid booking: the time is held at Checkout with no guest and nothing emailed until the
// payment; then the success page books it once, with the receipt, terms, waiver and VAT note.
import type { Page } from '@playwright/test';
import { calendar, type Coordinates, expect, mailbox, test } from '@tonalisk-labs/kit/testing';

import site from '../../../marcoferra.config.json' with { type: 'json' };
import { basic, sentTo, services } from '../support/api.ts';
import { icsLines, type Mail, mailOf } from '../support/mail.ts';
import { fakeStripe, holdOnPage, PAID, PAID_PATH, PRICE, sessionOf } from '../support/payment.ts';
import type { Visit } from '../support/visit.ts';

// Covers: event_types.price_minor, event_types.currency, event_types.title, booking.refund_until_hours

const RECEIPT = [
  `Paid: €${(PAID.price_minor / 100).toFixed(2)} by card.`,
  'IVA, regime de isenção, art. 53.º do CIVA.',
  'Your receipt from Stripe: ',
  `within ${String(site.booking.withdrawal_days)} days of booking, or at least ${String(site.booking.refund_until_hours)} hours before the start, refunds the payment in full`,
  'you lose the right of withdrawal once it has been given in full',
];

interface Held extends Visit {
  readonly session: string;
  /** The calendar event that holds the time. */
  readonly id: string;
}

/** From the type's card on /book/ to Stripe's Checkout, the time held for the payment. */
const holdFromTheList = async (page: Page, stack: Coordinates): Promise<Held> => {
  await page.goto('/book/');
  await expect(page.getByRole('article', { name: PAID.title })).toContainText(PRICE);
  await page.getByRole('link', { name: `Choose a time for: ${PAID.title}` }).click();
  await expect(page).toHaveURL(PAID_PATH);
  const held = await holdOnPage(page, stack, 'paid');
  await expect(page).toHaveURL(/\/checkout\/cs_test_\w+$/u);
  const session = sessionOf(page.url());
  const { client_reference_id: id } = await fakeStripe(stack).session(session);
  return Object.assign(structuredClone(held), { session, id: String(id) });
};

/** Held, not booked: no guest on the event, and nothing in the visitor's inbox. */
const expectHeldOnly = async (stack: Coordinates, held: Held): Promise<void> => {
  const { google, mailpit } = services(stack);
  expect(await calendar(google).event(held.id), 'held with no guest').toMatchObject({
    status: 'confirmed',
    attendees: [],
    summary: `Awaiting payment: ${PAID.title}`,
    extendedProperties: { private: { mfState: 'held', waiver: 'true' } },
  });
  expect(await sentTo(mailpit, held.guest.email), 'nothing before the payment').toEqual([]);
};

/** The invitation: its receipt paragraphs, and the meeting's own calendar lines. */
const expectInvitation = ({ text, invite }: Mail, start: string): void => {
  for (const line of RECEIPT) {
    expect(text, line).toContain(line);
  }
  const end = Date.parse(start) + PAID.duration_minutes * 60_000;
  expect(icsLines(invite)).toEqual(
    expect.arrayContaining([
      'METHOD:REQUEST',
      `SUMMARY:${PAID.summary}`,
      `DTSTART:${basic(start)}`,
      `DTEND:${basic(end)}`,
    ]),
  );
};

test('a visitor pays at Checkout and only then is invited, with the receipt', async ({
  page,
  stack,
}) => {
  const { google, mailpit } = services(stack);
  const held = await holdFromTheList(page, stack);
  await expectHeldOnly(stack, held);

  const arriving = mailbox(mailpit).waitFor(held.guest.email);
  await page.getByRole('button', { name: 'Pay', exact: true }).click();
  await expect(page).toHaveURL(`${PAID_PATH}done/?session_id=${held.session}`);
  await expect(page.getByRole('heading', { name: 'Check your email' })).toBeFocused();
  expectInvitation(await mailOf(mailpit, await arriving), held.start);

  expect(await calendar(google).event(held.id)).toMatchObject({
    summary: PAID.summary,
    attendees: [{ email: held.guest.email, displayName: held.guest.name }],
    extendedProperties: { private: { mfState: 'confirmed' } },
  });
  const completed = await fakeStripe(stack).delivered('checkout.session.completed', held.session);
  expect(completed).toEqual([200]);
  expect(await sentTo(mailpit, held.guest.email), 'invited once').toHaveLength(1);
});

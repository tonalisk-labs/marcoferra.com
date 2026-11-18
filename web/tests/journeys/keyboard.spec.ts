// Keyboard only: Tab reaches the calendar, the arrows and Enter pick the day and the time, and
// Enter sends the form.
import type { Locator, Page } from '@playwright/test';
import { expect, test } from '@tonalisk-labs/kit/testing';

import { timeOf } from '../../src/booking/times.ts';
import { claimStart, pastFillTime, visitor } from '../support/api.ts';
import { MEETING_PATH } from '../support/meeting.ts';
import { calendarShown, dayButton, zoneOf } from '../support/ui.ts';

interface Presses {
  readonly key: string;
  readonly reached: () => Promise<boolean>;
  readonly left: number;
}

/** Presses the key until `reached` holds, at most `left` more times. */
const pressUntil = async (page: Page, { key, reached, left }: Presses): Promise<void> => {
  if (await reached()) {
    return;
  }
  if (left === 0) {
    throw new Error(`${key} never reached its target`);
  }
  await page.keyboard.press(key);
  await pressUntil(page, { key, reached, left: left - 1 });
};

const focused = (target: Locator) => async (): Promise<boolean> =>
  (await target.and(target.page().locator(':focus')).count()) > 0;

/** Moves to the start's day with the arrows and opens it, then checks its time the same way. */
const chooseByKeys = async (page: Page, tab: string, start: string): Promise<void> => {
  const zone = await zoneOf(page);
  // A keyboard user waits for the calendar too: before it is drawn, Tab has nothing to reach.
  await calendarShown(page);
  const days = page.getByRole('grid').getByRole('button');
  await pressUntil(page, { key: tab, reached: focused(days), left: 40 });
  const day = dayButton(page, start, zone);
  await pressUntil(page, { key: 'ArrowRight', reached: focused(day), left: 150 });
  await page.keyboard.press('Enter');
  await pressUntil(page, { key: tab, reached: focused(page.getByRole('radio')), left: 10 });
  // Arrows move within the group (WebKit's stop at its ends), and Space checks the focused time;
  // a day's grid holds up to (last end - first start) / step times, 14 on the committed one.
  const time = page.getByRole('radio', { name: timeOf(start, zone), exact: true });
  await pressUntil(page, { key: 'ArrowDown', reached: focused(time), left: 24 });
  await page.keyboard.press('Space');
  await expect(time).toBeChecked();
};

test('a visitor books with the keyboard alone', async ({ page, stack, browserName, row }) => {
  const handheld = row !== undefined && (row.form === 'phone' || row.form === 'tablet');
  test.skip(handheld, 'a keyboard journey');
  // WebKit, like Safari, moves focus to buttons and links only with Option held.
  const tab = browserName === 'webkit' ? 'Alt+Tab' : 'Tab';
  const start = await claimStart(stack);
  const guest = visitor('keyboard');
  await page.goto(MEETING_PATH);
  await chooseByKeys(page, tab, start);
  const name = page.getByLabel('Name', { exact: true });
  await pressUntil(page, { key: tab, reached: focused(name), left: 5 });
  await page.keyboard.type(guest.name);
  await page.keyboard.press(tab);
  await page.keyboard.type(guest.email);
  await pastFillTime();
  await page.keyboard.press('Enter');
  await expect(page.getByRole('heading', { name: 'Check your email' })).toBeFocused();
});

import { expect, type Locator, type Page } from '@playwright/test';

import { longDayOf, timeOf } from '../../src/booking/times.ts';

/** The zone the page detected, which the page shows every time in. */
export const zoneOf = async (page: Page): Promise<string> =>
  page.evaluate(() => new Intl.DateTimeFormat().resolvedOptions().timeZone);

/** Waits for the island to have read the free times and drawn its calendar. */
export const calendarShown = async (page: Page): Promise<void> => {
  await expect(page.getByRole('grid')).toBeVisible();
};

/** A day's button in the calendar, by the long day its name holds. */
export const dayButton = (page: Page, start: string, zone: string): Locator =>
  page.getByRole('grid').getByRole('button', { name: longDayOf(start, zone) });

/** Moves the calendar on month by month, each turn awaited, until the day's button shows. */
const monthOf = async (page: Page, day: Locator, turns = 0): Promise<void> => {
  if ((await day.count()) > 0) {
    return;
  }
  const next = page.getByRole('button', { name: 'Go to the Next Month' });
  if (turns > 4 || (await next.getAttribute('aria-disabled')) === 'true') {
    throw new Error('the calendar ends before the day wanted');
  }
  const grid = page.getByRole('grid');
  const shown = (await grid.getAttribute('aria-label')) ?? '';
  await next.click();
  await expect(grid).not.toHaveAccessibleName(shown);
  await monthOf(page, day, turns + 1);
};

export const chooseDay = async (page: Page, start: string, zone: string): Promise<void> => {
  const day = dayButton(page, start, zone);
  await calendarShown(page);
  await monthOf(page, day);
  await day.click();
};

export const chooseStart = async (page: Page, start: string, zone: string): Promise<void> => {
  await chooseDay(page, start, zone);
  await page.getByRole('radio', { name: timeOf(start, zone), exact: true }).check();
};

export const fillDetails = async (page: Page, name: string, email: string): Promise<void> => {
  await page.getByLabel('Name', { exact: true }).fill(name);
  await page.getByLabel('Email', { exact: true }).fill(email);
};

export const waive = async (page: Page): Promise<void> => {
  await page.getByRole('checkbox', { name: /withdrawal period/u }).check();
};

/** Sends a paid type's form once its fill time has passed: "Pay €40 and book". */
export const pay = async (
  page: Page,
  { price, wait }: { readonly price: string; readonly wait: () => Promise<void> },
): Promise<void> => {
  await wait();
  await page.getByRole('button', { name: `Pay ${price} and book` }).click();
};

/** Sends the form once its fill time has passed. */
export const send = async (page: Page, wait: () => Promise<void>): Promise<void> => {
  await wait();
  await page.getByRole('button', { name: 'Book the meeting' }).click();
};

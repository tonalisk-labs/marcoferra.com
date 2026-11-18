// A shared link's utm_source goes on to the meeting type the visitor chooses.
import type { Locator, Page } from '@playwright/test';
import { expect, test } from '@tonalisk-labs/kit/testing';

import { ROLE_TYPE } from '../src/booking/brief.ts';
import { eventTypes } from '../src/booking/event-types.ts';

const MENU = ['Home', 'CV', 'Work with me'];

/** A meeting type card's link, by the name it is read out with. */
const choose = (title: string): string => `Choose a time for: ${title}`;

/** A card's link leads to its type's page with the query the visitor came with. */
const leadsTo = async (link: Locator, slug: string): Promise<void> => {
  await expect(link).toHaveJSProperty('pathname', `/book/${slug}/`);
  await expect(link).toHaveJSProperty('search', '?utm_source=linkedin');
};

/** The main menu: a phone shows it behind a button, a wider screen as a row. */
const mainMenu = async (page: Page): Promise<Locator> => {
  const button = page.getByRole('button', { name: 'Menu' });
  if (await button.isVisible()) {
    await button.click();
    return page.getByRole('navigation', { name: 'Menu' });
  }
  return page.getByRole('navigation', { name: 'Main' });
};

test('the main menu reads Home, CV and Work with me, and leads there', async ({ page }) => {
  await page.goto('/cv/');
  const menu = await mainMenu(page);
  await expect(menu.getByRole('link')).toHaveText(MENU);
  await menu.getByRole('link', { name: 'Work with me', exact: true }).click();
  await expect(page).toHaveURL('/work-with-me/');
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('Work with me');
  const again = await mainMenu(page);
  await expect(again.getByRole('link', { name: 'Work with me', exact: true })).toHaveAttribute(
    'aria-current',
    'page',
  );
});

test('Work with me opens a door for roles and one for advisory', async ({ page }) => {
  const kinds = eventTypes();
  const role = kinds.find(({ slug }) => slug === ROLE_TYPE);
  if (role === undefined) {
    throw new Error(`marcoferra.config.json lists no ${ROLE_TYPE}`);
  }
  await page.goto('/work-with-me/?utm_source=linkedin');
  const roles = page.getByRole('region', { name: 'Roles' });
  await leadsTo(roles.getByRole('link', { name: choose(role.title) }), ROLE_TYPE);
  await expect(roles.getByRole('article', { name: role.title })).toContainText('Free');
  const cv = roles.getByRole('article', { name: 'Full CV on request' });
  await expect(cv.getByRole('link', { name: 'hello@marcoferra.com' })).toHaveAttribute(
    'href',
    'mailto:hello@marcoferra.com?subject=Full%20CV',
  );
  const advisory = page.getByRole('region', { name: 'Advisory' });
  const others = kinds.filter(({ slug }) => slug !== ROLE_TYPE);
  await expect(advisory.getByRole('article')).toHaveCount(others.length);
  await Promise.all(
    others.map(async (kind) =>
      leadsTo(advisory.getByRole('link', { name: choose(kind.title) }), kind.slug),
    ),
  );
  await expect(advisory.getByRole('link', { name: choose(role.title) })).toHaveCount(0);
});

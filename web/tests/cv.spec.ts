// The PDF is set in Type0 fonts with Unicode maps, so parsers extract its text as text.
import { expect, test } from '@tonalisk-labs/kit/testing';

import { employerNamed } from './support/employer.ts';

test('the CV names no employer for the current role and offers the full CV on request', async ({
  page,
}) => {
  await page.goto('/cv/');
  const experience = page.getByRole('region', { name: 'Experience' });
  await expect(experience.getByRole('heading', { level: 3 }).first()).toHaveText(
    'Programme Manager, Global media and technology company',
  );
  const request = page.locator('p', { hasText: 'Full CV on request' });
  await expect(request.getByRole('link', { name: 'hello@marcoferra.com' })).toHaveAttribute(
    'href',
    'mailto:hello@marcoferra.com?subject=Full%20CV',
  );
  await expect(page.getByRole('link', { name: 'Download the CV (PDF)' })).toHaveAttribute(
    'href',
    '/Marco-Ferra-CV.pdf',
  );
});

test('the CV ends with the two doors of Work with me', async ({ page }) => {
  await page.goto('/cv/');
  const next = page.getByRole('region', { name: 'Next step' });
  await expect(next.getByRole('link', { name: 'Roles' })).toHaveAttribute(
    'href',
    '/work-with-me/#roles',
  );
  await expect(next.getByRole('link', { name: 'Advisory' })).toHaveAttribute(
    'href',
    '/work-with-me/#advisory',
  );
});

test('the downloadable CV is PDF 2.0, parseable, and names no employer or phone number', async ({
  page,
}) => {
  const response = await page.request.get('/Marco-Ferra-CV.pdf');
  expect(response.status()).toBe(200);
  // The header, the catalog, the font dictionaries, the outline and the information are plain
  // strings; the page text is glyph codes in compressed streams.
  const body = await response.body();
  const bytes = body.toString('latin1');
  expect(bytes.startsWith('%PDF-2.0')).toBe(true);
  // PDF 2.0 reads its metadata from the XMP packet the catalog points to.
  const catalog = bytes.split('\n').find((line) => line.includes('/Type /Catalog')) ?? '';
  expect(catalog).toMatch(/\/Metadata \d+ 0 R/u);
  expect(bytes).toContain('/StructTreeRoot');
  expect(bytes).not.toContain('/Type3');
  const fonts = bytes.match(/\/Subtype \/Type0/gu) ?? [];
  expect(fonts.length).toBeGreaterThan(0);
  expect(bytes.match(/\/ToUnicode/gu) ?? []).toHaveLength(fonts.length);
  expect(bytes).toContain('/Title (Programme Manager, Global media and technology company)');
  expect(employerNamed(bytes)).toEqual([]);
  expect(bytes).not.toContain('961 787');
});

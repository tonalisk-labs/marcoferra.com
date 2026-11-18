// A sitemap page's date while the repository is one root commit: the date kept from the history
// before it, until a later commit changes the page's sources, when git's date stands again.
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { test, type TestContext } from 'node:test';

import { chosenDate, datedAgainst, datedSince } from './page-dates.mjs';

const KEPT = '2026-11-04T18:31:02+00:00';
// An hour east of UTC, which git writes back as it was given, where UTC may read as Z.
const AT_ROOT = '2026-11-11T22:00:00+01:00';
const LATER = '2026-11-12T10:30:00+01:00';
const SITE = 'https://www.example.com';

/** A repository whose root commit holds a.md and b.md, and whose next commit changes b.md alone;
 *  c.md is there but never committed. Removed when the test ends. */
const repository = (context: TestContext): string => {
  const dir = mkdtempSync(path.join(tmpdir(), 'page-dates-'));
  context.after(() => {
    rmSync(dir, { recursive: true, force: true });
  });
  // Without the user's own settings, which may sign each commit, and with a fixed author and date.
  const git = (date: string, ...args: string[]): void => {
    execFileSync('git', args, {
      cwd: dir,
      env: Object.assign(structuredClone(process.env), {
        GIT_CONFIG_GLOBAL: '/dev/null',
        GIT_AUTHOR_NAME: 'Test',
        GIT_AUTHOR_EMAIL: 'test@example.com',
        GIT_AUTHOR_DATE: date,
        GIT_COMMITTER_NAME: 'Test',
        GIT_COMMITTER_EMAIL: 'test@example.com',
        GIT_COMMITTER_DATE: date,
      }),
      stdio: 'ignore',
    });
  };
  git(AT_ROOT, 'init', '-q', '--initial-branch=main');
  writeFileSync(path.join(dir, 'a.md'), 'a\n');
  writeFileSync(path.join(dir, 'b.md'), 'b\n');
  git(AT_ROOT, 'add', 'a.md', 'b.md');
  git(AT_ROOT, 'commit', '-q', '-m', 'feat: the root');
  writeFileSync(path.join(dir, 'b.md'), 'b, changed\n');
  git(LATER, 'commit', '-q', '-am', 'fix: b changes');
  writeFileSync(path.join(dir, 'c.md'), 'c\n');
  return dir;
};

/** Each page is its own file, by its first path segment; the home page has none. */
const sourcesOf = (pathname: string): readonly string[] => {
  const name = pathname.split('/').find(Boolean) ?? '';
  return name === '' ? [] : [`${name}.md`];
};

await test('a page whose sources last changed in the root takes the date kept for it', () => {
  assert.equal(chosenDate({ kept: KEPT, changedAtRoot: true, fromGit: AT_ROOT }), KEPT);
});

await test('a page changed since the root takes git’s date, kept or not', () => {
  assert.equal(chosenDate({ kept: KEPT, changedAtRoot: false, fromGit: LATER }), LATER);
  assert.equal(chosenDate({ kept: undefined, changedAtRoot: false, fromGit: LATER }), LATER);
});

await test('a page with no date kept takes git’s, even at the root', () => {
  assert.equal(chosenDate({ kept: undefined, changedAtRoot: true, fromGit: AT_ROOT }), AT_ROOT);
  assert.equal(chosenDate({ kept: undefined, changedAtRoot: true, fromGit: undefined }), undefined);
});

await test('the sitemap dates each page by its sources in the repository', (context) => {
  const dated = datedAgainst(
    { [`${SITE}/a/`]: KEPT, [`${SITE}/b/`]: KEPT },
    repository(context),
    sourcesOf,
  );
  assert.deepEqual(dated({ url: `${SITE}/a/` }), { url: `${SITE}/a/`, lastmod: KEPT });
  assert.deepEqual(dated({ url: `${SITE}/b/` }), { url: `${SITE}/b/`, lastmod: LATER });
  assert.deepEqual(dated({ url: `${SITE}/c/` }), { url: `${SITE}/c/` });
  assert.deepEqual(dated({ url: `${SITE}/` }), { url: `${SITE}/` });
});

await test('a page at the root with no date kept takes the root’s', (context) => {
  const dated = datedAgainst({}, repository(context), sourcesOf);
  assert.deepEqual(dated({ url: `${SITE}/a/` }), { url: `${SITE}/a/`, lastmod: AT_ROOT });
});

await test('a shallow clone is refused, since its oldest commit only looks like the root', (context) => {
  const full = repository(context);
  const clone = mkdtempSync(path.join(tmpdir(), 'page-dates-shallow-'));
  context.after(() => {
    rmSync(clone, { recursive: true, force: true });
  });
  execFileSync('git', ['clone', '-q', '--depth', '1', `file://${full}`, clone], {
    stdio: 'ignore',
  });
  assert.throws(() => datedAgainst({}, clone, sourcesOf), /shallow clone/u);
});

await test('the site’s own kept dates date nothing a page has no source for', (context) => {
  const dated = datedSince(repository(context), sourcesOf);
  assert.deepEqual(dated({ url: `${SITE}/` }), { url: `${SITE}/` });
});

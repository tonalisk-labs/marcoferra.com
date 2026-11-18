// A sitemap page's date while the repository's history begins at one root commit: the date
// page-dates.json kept from the history before it, until a later commit changes the page's sources.
import { execFileSync } from 'node:child_process';

import { lastChanged } from '@tonalisk-labs/kit/astro';

import dates from './page-dates.json' with { type: 'json' };

/** @type {Readonly<Record<string, string>>} */
const KEPT = dates;

/** The lastmod a page takes: its kept date while its sources last changed in the root commit, else
 *  git's date, or none.
 *  @param {{ kept: string | undefined, changedAtRoot: boolean, fromGit: string | undefined }} page */
export const chosenDate = ({ kept, changedAtRoot, fromGit }) =>
  changedAtRoot && kept !== undefined ? kept : fromGit;

/** Whether the last commit to change `files` has no parent, so no history before it dates them.
 *  @param {string} cwd
 *  @param {readonly string[]} files */
const changedAtRoot = (cwd, files) => {
  const line = execFileSync('git', ['log', '-1', '--format=%H %P', '--', ...files], {
    cwd,
    encoding: 'utf8',
  }).trim();
  return line !== '' && !line.includes(' ');
};

/** Whether `cwd` is a shallow clone, whose oldest commit looks like a root however old it is.
 *  @param {string} cwd */
const shallow = (cwd) =>
  execFileSync('git', ['rev-parse', '--is-shallow-repository'], {
    cwd,
    encoding: 'utf8',
  }).trim() === 'true';

/** As datedSince, with the kept dates it reads named.
 *  @param {Readonly<Record<string, string>>} kept
 *  @param {string} root
 *  @param {(pathname: string) => readonly string[]} sourcesOf
 *  @returns {<Item extends { url: string, lastmod?: string }>(item: Item) => Item} */
export const datedAgainst = (kept, root, sourcesOf) => {
  if (shallow(root)) {
    throw new Error(`${root} is a shallow clone, so its pages can't be dated: fetch its history`);
  }
  return (item) => {
    const files = sourcesOf(new URL(item.url).pathname);
    const lastmod = chosenDate({
      kept: kept[item.url],
      changedAtRoot: files.length > 0 && changedAtRoot(root, files),
      fromGit: lastChanged(root, [...files]),
    });
    return lastmod === undefined ? item : Object.assign(item, { lastmod });
  };
};

/** A sitemap `serialize`, as kit's datedBy, that takes a page's kept date while the root commit is
 *  the last to change its sources: `sourcesOf` names the files, from `root`, that make a page.
 *  @param {string} root
 *  @param {(pathname: string) => readonly string[]} sourcesOf */
export const datedSince = (root, sourcesOf) => datedAgainst(KEPT, root, sourcesOf);

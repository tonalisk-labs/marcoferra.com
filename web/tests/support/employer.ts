// The current employer stays out of everything public. Its names are private too, so they come
// from PRIVATE_NAMES, one regular expression: a secret in CI, mise.local.toml on a workstation.
const source = process.env['PRIVATE_NAMES'] ?? '';

/** Each name of the current employer that `text` carries, as a failure message. */
export const employerNamed = (text: string): string[] => {
  if (source === '') {
    return ['PRIVATE_NAMES is unset, so no page can be checked for the employer'];
  }
  const found = [...text.matchAll(new RegExp(source, 'gu'))].map(([name]) => name);
  return [...new Set(found)].map((name) => `names the current employer: ${name}`);
};

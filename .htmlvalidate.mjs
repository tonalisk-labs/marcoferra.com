// Every built page against html-validate's recommended and document presets. What's relaxed here
// is only what Astro writes, which the site can't change.
export default {
  extends: ['html-validate:recommended', 'html-validate:document'],
  elements: [
    'html5',
    // Astro writes its islands' <style> into the body, just before a page's first island.
    { style: { flow: true } },
  ],
  rules: {
    // Astro's compressed output keeps a space before some line breaks.
    'no-trailing-whitespace': 'off',
    // Integrity for every cross-origin resource. The site's own files are served with the page.
    'require-sri': ['error', { target: 'crossorigin' }],
  },
};

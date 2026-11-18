import { jsonLdScript } from '@tonalisk-labs/kit/theme';
import type { SoftwareApplication, Thing } from 'schema-dts';

export const LINKEDIN = 'https://www.linkedin.com/in/maferra';
export const EMAIL = 'hello@marcoferra.com';

/** The same person's public mentor profiles. */
const MENTOR_PROFILES = [
  'https://mentorcruise.com/mentor/marcoferra/',
  'https://mentor.sh/mentors/marcoferra',
  'https://www.growthmentor.com/mentors/marco-ferra',
  'https://www.coachbase.io/coach/marco-ferra',
];

const KNOWS_ABOUT = [
  'Agile delivery',
  'Lean Six Sigma',
  'DORA metrics',
  'Value Stream Mapping',
  'AI-assisted software delivery',
  'SAFe',
  'BPMN',
];

const personId = (site: URL | undefined): string => `${new URL('/', site).href}#person`;

/** The product the author builds and runs, related to him as its creator: not one of his
 *  profiles, so not in `sameAs`. As astronotes.eu describes itself. */
export const astroNotes = (site: URL | undefined): SoftwareApplication => ({
  '@type': 'SoftwareApplication',
  name: 'Astro Notes',
  applicationCategory: 'ProductivityApplication',
  operatingSystem: 'Web',
  url: 'https://www.astronotes.eu/',
  offers: { '@type': 'Offer', price: '0', priceCurrency: 'EUR' },
  creator: { '@id': personId(site) },
});

/** The author and the site on every page, and whatever the page adds about itself. The author
 *  names no employer. */
export const siteJsonLd = (site: URL | undefined, extra: readonly Thing[] = []): string => {
  const home = new URL('/', site).href;
  return jsonLdScript({
    '@context': 'https://schema.org',
    '@graph': [
      {
        '@type': 'Person',
        '@id': personId(site),
        name: 'Marco Ferra',
        jobTitle: 'Programme Manager',
        email: `mailto:${EMAIL}`,
        url: home,
        sameAs: [LINKEDIN, ...MENTOR_PROFILES, 'https://www.askmeonequestion.com/'],
        knowsAbout: KNOWS_ABOUT,
        address: { '@type': 'PostalAddress', addressLocality: 'London', addressCountry: 'GB' },
      },
      {
        '@type': 'WebSite',
        '@id': `${home}#site`,
        name: 'Marco Ferra',
        url: home,
        inLanguage: 'en-GB',
        author: { '@id': personId(site) },
      },
      ...extra,
    ],
  });
};

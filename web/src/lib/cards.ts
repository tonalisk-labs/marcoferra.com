// The social cards in public/og/, by file name: their text, and its alt text for each page.
// scripts/cards.ts draws them from this.

export interface Card {
  readonly title: string;
  /** The line under the title, its parts set apart by middle dots. */
  readonly details: readonly string[];
  readonly alt: string;
}

export const CARDS = {
  home: {
    title: 'Engineering excellence and transformation',
    details: ['Programme Manager', 'London'],
    alt: 'Marco Ferra: Engineering excellence and transformation. Programme Manager, London.',
  },
  cv: {
    title: 'CV: Programme Manager',
    details: ['Agile delivery', 'Lean Six Sigma', 'DevOps and DORA metrics', 'SAFe'],
    alt: 'Marco Ferra, CV: Programme Manager. Agile delivery, Lean Six Sigma, DevOps and DORA metrics, SAFe.',
  },
  book: {
    title: 'Book a meeting',
    details: ['Choose a day and a time for a video call'],
    alt: 'Marco Ferra: Book a meeting. Choose a day and a time for a video call.',
  },
  'work-with-me': {
    title: 'Work with me',
    details: ['Roles and advisory', 'Full CV on request'],
    alt: 'Marco Ferra: Work with me. Roles and advisory, full CV on request.',
  },
  privacy: {
    title: 'Privacy policy',
    details: ['What a booking collects, who processes it, and for how long'],
    alt: 'Marco Ferra: Privacy policy. What a booking collects, who processes it, and for how long.',
  },
  terms: {
    title: 'Terms of use',
    details: ['The terms for booking a meeting on marcoferra.com'],
    alt: 'Marco Ferra: Terms of use. The terms for booking a meeting on marcoferra.com.',
  },
} as const satisfies Record<string, Card>;

export type CardName = keyof typeof CARDS;

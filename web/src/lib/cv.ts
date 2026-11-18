// The CV in two editions. Public is /cv/ and its PDF. Recruiter is the full CV on request, printed
// from an overlay kept outside this repo, and never published.
export type Edition = 'public' | 'recruiter';

interface Role {
  readonly company: string;
  readonly role: string;
  readonly sector: string;
  readonly location: string;
  /** `YYYY-MM`. */
  readonly start: string;
  /** `YYYY-MM`, or `present`. */
  readonly end: string;
  readonly part_time?: boolean;
  readonly context: string;
  /** The titles held on the way up, first to last. */
  readonly steps?: readonly string[];
  readonly highlights: readonly string[];
}

interface Item {
  readonly title: string;
  readonly detail: string;
  readonly url?: string;
  /** The link's words on /cv/. */
  readonly cta?: string;
}

export interface Cv {
  readonly name: string;
  readonly title: string;
  readonly tagline: string;
  readonly location: string;
  readonly email: string;
  readonly linkedin: string;
  readonly website: string;
  readonly summary: string;
  readonly achievements: readonly string[];
  readonly experience: readonly Role[];
  readonly alongside: readonly Item[];
  readonly speaking: readonly Item[];
  readonly publications: readonly Item[];
  readonly skills: readonly { group: string; items: readonly string[] }[];
  readonly education: readonly { qualification: string; institution: string; year: number }[];
  readonly certifications: readonly { name: string; detail: string; pdf?: string }[];
  readonly languages: readonly string[];
}

/** A role as one edition shows it. */
interface Job {
  readonly company: string;
  readonly role: string;
  readonly sector: string;
  readonly location: string;
  readonly start: string;
  readonly end: string;
  readonly partTime: boolean;
  readonly context: string;
  readonly steps: readonly string[];
  readonly highlights: readonly string[];
}

/** The CV as one edition shows it: every choice between the two is made here. */
export interface CvEdition extends Omit<Cv, 'experience'> {
  readonly edition: Edition;
  /** Only the recruiter edition carries a phone number. */
  readonly phone: string | undefined;
  readonly experience: readonly Job[];
}

// US English, since en-GB writes September as "Sept", which some CV parsers miss.
const MONTH = new Intl.DateTimeFormat('en-US', {
  month: 'short',
  year: 'numeric',
  timeZone: 'UTC',
});

/** `Oct 2022`, or `Present`. */
export const monthOf = (value: string): string =>
  value === 'present' ? 'Present' : MONTH.format(new Date(`${value}-01T00:00:00Z`));

/** `Oct 2022 to Present`: words, not a dash, which the copy rules ban and parsers read alike. */
export const periodOf = ({ start, end }: { start: string; end: string }): string =>
  `${monthOf(start)} to ${monthOf(end)}`;

export const yearOf = (value: string): string => value.slice(0, 4);

const SECTIONS = [
  'achievements',
  'experience',
  'alongside',
  'speaking',
  'publications',
  'skills',
  'education',
  'certifications',
  'languages',
];

const isCv = (value: unknown): value is Cv =>
  typeof value === 'object' &&
  value !== null &&
  'name' in value &&
  typeof value.name === 'string' &&
  SECTIONS.every((key) => Array.isArray(Reflect.get(value, key)));

/** The frontmatter of cv.md; a missing section fails the build, not a visitor. */
export const asCv = (frontmatter: unknown): Cv => {
  if (!isCv(frontmatter)) {
    throw new TypeError('cv.md lacks a name or one of its sections');
  }
  return frontmatter;
};

/** What the recruiter edition says in place of the public one. A role is matched by its start. */
export interface Recruiter {
  readonly tagline: string;
  readonly phone: string;
  readonly summary: string;
  readonly achievements: readonly string[];
  readonly roles: readonly {
    readonly start: string;
    readonly company?: string;
    readonly context?: string;
    readonly highlights?: readonly string[];
  }[];
}

const isStrings = (value: unknown): value is readonly string[] =>
  Array.isArray(value) && value.every((item) => typeof item === 'string');

const isRecruiter = (value: unknown): value is Recruiter =>
  typeof value === 'object' &&
  value !== null &&
  ['tagline', 'phone', 'summary'].every((key) => typeof Reflect.get(value, key) === 'string') &&
  isStrings(Reflect.get(value, 'achievements')) &&
  Array.isArray(Reflect.get(value, 'roles'));

/** The overlay's JSON; one missing a field fails the print, not a reader. */
export const asRecruiter = (json: unknown): Recruiter => {
  if (!isRecruiter(json)) {
    throw new TypeError(
      'the recruiter overlay lacks a tagline, phone, summary, achievements or roles',
    );
  }
  return json;
};

type RoleOverlay = Recruiter['roles'][number];

const jobOf = (role: Role, overlay: Partial<RoleOverlay>): Job => ({
  company: overlay.company ?? role.company,
  role: role.role,
  sector: role.sector,
  location: role.location,
  start: role.start,
  end: role.end,
  partTime: role.part_time ?? false,
  context: overlay.context ?? role.context,
  steps: role.steps ?? [],
  highlights: overlay.highlights ?? role.highlights,
});

/** What one edition says where the two differ. */
interface Wording {
  readonly edition: Edition;
  readonly tagline: string;
  readonly phone: string | undefined;
  readonly summary: string;
  readonly achievements: readonly string[];
  readonly roles: readonly RoleOverlay[];
}

const shown = (cv: Cv, wording: Wording): CvEdition => {
  const overlayOf = (role: Role): Partial<RoleOverlay> =>
    wording.roles.find(({ start }) => start === role.start) ?? {};
  return {
    edition: wording.edition,
    name: cv.name,
    title: cv.title,
    tagline: wording.tagline,
    location: cv.location,
    email: cv.email,
    phone: wording.phone,
    linkedin: cv.linkedin,
    website: cv.website,
    summary: wording.summary,
    achievements: wording.achievements,
    experience: cv.experience.map((role) => jobOf(role, overlayOf(role))),
    alongside: cv.alongside,
    speaking: cv.speaking,
    publications: cv.publications,
    skills: cv.skills,
    education: cv.education,
    certifications: cv.certifications,
    languages: cv.languages,
  };
};

/** The public CV, or with `recruiter` the full one. */
export const editionOf = (cv: Cv, recruiter?: Recruiter): CvEdition =>
  recruiter === undefined
    ? shown(cv, {
        edition: 'public',
        tagline: cv.tagline,
        phone: undefined,
        summary: cv.summary,
        achievements: cv.achievements,
        roles: [],
      })
    : shown(cv, {
        edition: 'recruiter',
        tagline: recruiter.tagline,
        phone: recruiter.phone,
        summary: recruiter.summary,
        achievements: recruiter.achievements,
        roles: recruiter.roles,
      });

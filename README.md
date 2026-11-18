# marcoferra.com

[marcoferra.com](https://www.marcoferra.com) is Marco Ferra's site: how to work with him, his CV on
request, and a page to book a call. `web/` is the site, built with Astro. `server/` is its booking
API, a Rust service on AWS Lambda.

## Run it

Install [mise](https://mise.jdx.dev), then:

```sh
mise install
npm ci
mise run generate
npm run dev -w web
```

`mise run check` runs every check a pull request does. `mise run e2e` runs the site and its API
in every browser of the matrix, against stand-ins for Google Calendar, email and Stripe.

## Workflows

The repo is public, so its Actions minutes are free. Each workflow calls kit's own, pinned to a
commit, and every job holds only the permissions it needs.

- `ci.yml`, on each pull request: kit's `checks.yml`, `mise run check` and the pull request's
  title.
- `release.yml`, on each push to `main`: kit's `release.yml`, then its `pages.yml`. release-please
  keeps a release pull request open, and merging it runs the release check, attests its files and
  attaches them, and puts the site on Pages.

The infra repo deploys the booking API and the CloudFront edge from a release, once its pin
there moves.

## Licence

[MIT](LICENSE)

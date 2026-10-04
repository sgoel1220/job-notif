# Initial company expansion audit — 2026-10-03

**Historical snapshot below.** Subsequent root-cause investigation raised CSV coverage to **117/196** and registry coverage to **203/282**, leaving **79** disabled CSV companies. See [company-root-causes.md](company-root-causes.md) for current outcomes and `companies.csv` for per-company ownership, evidence, and next actions.

Coverage means an enabled source mapping, not guaranteed current jobs, remote eligibility, or India eligibility. Existing enabled feeds were not all reverified in this pass.

| Scope | Before | After |
|---|---:|---:|
| CSV companies enabled | 40/196 | 82/196 |
| Entire registry enabled | 118/274 | 168/282 |
| Unresolved CSV companies | 156 | 114 |

Added 50 unique verified sources: 42 existing CSV employers and 8 new employers beyond both lists. The CSV is preserved as the original target list.

## Soham repository review

- Upstream: https://github.com/sohamb17/internship-watcher
- Reviewed commit: `21611e6add16618674b16ade6be9f2b73b7c240a`.
- `boards.json`: 101 mappings (44 Greenhouse, 4 Lever, 18 Ashby, 6 SmartRecruiters, 28 Workday, 1 Amazon). Eligible supported mappings were already enabled locally or covered by the CSV research set; no net-new supported mappings were imported from this snapshot.
- Amazon remains outside the implemented ATS adapters. Historical “live verified” documentation is not a current health check.
- This is curated company discovery, not an exhaustive employer index. Its US/internship filters must not be copied into this broader job search.
- Reusable pattern: structured registry seeds → official careers/hosted board identity → public API payload validation → dated evidence. Workday URLs require local `tenant/shard/site` conversion.

## Newly enabled sources

Every row passed a nonempty provider-shaped JSON check by the parent and has official careers-page or hosted-board employer evidence from Luna. Job counts are point-in-time feed counts, not filtered or globally deduplicated jobs.

| Company | Provider | Board | Jobs checked | Identity evidence |
|---|---|---|---:|---|
| 1Password | ashby | `1password` | 69 | https://jobs.ashbyhq.com/1password |
| Axon | greenhouse | `axon` | 487 | https://boards.greenhouse.io/axon |
| Baseten | ashby | `baseten` | 105 | https://www.baseten.co/resources/careers/ |
| Binance | lever | `binance` | 310 | https://jobs.lever.co/binance |
| Block | greenhouse | `block` | 227 | https://job-boards.greenhouse.io/block |
| Bungie | greenhouse | `bungie` | 2 | https://boards.greenhouse.io/bungie |
| Calendly | greenhouse | `calendly` | 10 | https://boards.greenhouse.io/calendly |
| Calm | greenhouse | `calm` | 2 | https://job-boards.greenhouse.io/calm |
| Canonical | greenhouse | `canonical` | 310 | https://job-boards.greenhouse.io/canonical |
| Caylent | greenhouse | `caylent` | 77 | https://boards.greenhouse.io/caylent |
| Circle | ashby | `circle` | 41 | https://jobs.ashbyhq.com/circle |
| CircleCI | greenhouse | `circleci` | 12 | https://boards.greenhouse.io/circleci |
| Close | ashby | `close` | 5 | https://jobs.ashbyhq.com/close |
| Collibra | greenhouse | `collibra` | 32 | https://job-boards.greenhouse.io/collibra |
| Consensys | ashby | `consensys` | 6 | https://jobs.ashbyhq.com/consensys |
| Convex | ashby | `convex-dev` | 13 | https://www.convex.dev/jobs |
| Customer.io | greenhouse | `customerio` | 29 | https://job-boards.greenhouse.io/customerio |
| Degreed | greenhouse | `degreed` | 2 | https://boards.greenhouse.io/degreed |
| Fastly | greenhouse | `fastly` | 43 | https://job-boards.greenhouse.io/fastly |
| Grafana | greenhouse | `grafanalabs` | 121 | https://boards.greenhouse.io/grafanalabs |
| Illumio | ashby | `illumio` | 65 | https://jobs.ashbyhq.com/illumio |
| Karat | greenhouse | `karat` | 10 | https://job-boards.greenhouse.io/karat |
| LangChain | ashby | `langchain` | 102 | https://www.langchain.com/careers |
| Mattermost | greenhouse | `mattermost` | 14 | https://job-boards.greenhouse.io/mattermost |
| Metabase | lever | `metabase` | 18 | https://jobs.lever.co/metabase |
| Mozilla | greenhouse | `mozilla` | 90 | https://boards.greenhouse.io/mozilla |
| Neon | ashby | `neon` | 4 | https://neon.com/careers |
| Netlify | greenhouse | `netlify` | 5 | https://www.netlify.com/careers/ |
| Okta | greenhouse | `okta` | 368 | https://job-boards.greenhouse.io/okta |
| OpenSea | ashby | `opensea` | 2 | https://jobs.ashbyhq.com/opensea |
| Pagerduty | greenhouse | `pagerduty` | 53 | https://job-boards.greenhouse.io/pagerduty |
| PandaDoc | greenhouse | `pandadoc` | 10 | https://job-boards.greenhouse.io/pandadoc |
| Plaid | ashby | `plaid` | 119 | https://jobs.ashbyhq.com/plaid |
| PostHog | ashby | `posthog` | 8 | https://posthog.com/careers |
| RelationalAI | greenhouse | `relationalai` | 3 | https://job-boards.greenhouse.io/relationalai |
| runZero | greenhouse | `runzero` | 1 | https://job-boards.greenhouse.io/runzero |
| SecurityScorecard | greenhouse | `securityscorecard` | 13 | https://job-boards.greenhouse.io/securityscorecard |
| Sentry | ashby | `sentry` | 41 | https://sentry.io/careers/ |
| SingleStore | greenhouse | `singlestore` | 40 | https://job-boards.greenhouse.io/singlestore |
| Stedi | ashby | `stedi` | 16 | https://jobs.ashbyhq.com/stedi |
| Temporal | ashby | `temporal` | 64 | https://temporal.io/careers |
| Thumbtack | ashby | `thumbtack` | 57 | https://jobs.ashbyhq.com/thumbtack |
| Together AI | greenhouse | `togetherai` | 77 | https://www.together.ai/careers |
| Toptal | lever | `toptal` | 32 | https://jobs.lever.co/toptal |
| Unqork | greenhouse | `unqork` | 1 | https://boards.greenhouse.io/unqork |
| Upwork | greenhouse | `upwork` | 7 | https://job-boards.greenhouse.io/upwork |
| Vercel | greenhouse | `vercel` | 85 | https://vercel.com/careers |
| Webflow | greenhouse | `webflow` | 25 | https://job-boards.greenhouse.io/webflow |
| Wikimedia | greenhouse | `wikimedia` | 11 | https://job-boards.greenhouse.io/wikimedia |
| Zapier | ashby | `zapier` | 11 | https://jobs.ashbyhq.com/zapier |

## Explicit unresolved CSV companies

These entries remain disabled, with no guessed provider or board. Unresolved is not proof of an unsupported ATS or no openings; it means this pass did not establish a defensible mapping.

- Cribl: both Greenhouse and Ashby expose branded nonempty feeds; official careers page did not establish which is authoritative. Neither enabled.
- Elastic: nonempty Greenhouse feed, but hosted-page identity check inconclusive; not enabled.
- Consensys Greenhouse board is branded MetaMask; rejected in favor of the verified Consensys Ashby board.
- CrowdStrike: Workday is supported, but the correct tenant/shard/site remains unverified.
- Deel: an empty guessed Ashby feed is insufficient identity evidence.

| Company | Registry note |
|---|---|
| 1047 Games | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| Acorns | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| AgentSync | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| Akamai | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| Alma | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| Almanac | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| AngelList | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| Angi | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| Apollo.io | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| Asapp | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| Ashby | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| Atlassian | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| Auth0 | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| Bird | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| Blockchain.com | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| Bloom Tech | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| BuySellAds | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| Cameo | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| Cerebral | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| Chain | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| Chainlink Labs | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| Chess.com | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| Clearbit | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| Cloudbees | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| Cloudless Labs | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| Commsor | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| Coqui | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| Cribl | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| CrowdStrike | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| Dapper Labs | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| DappRadar | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| DataChain | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| Deel | Unresolved: research found Ashby careers evidence, but board slug was not verified. |
| DigitalOcean | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| DocuSign | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| Elastic | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| EvenUp | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| Ginger | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| GitHub | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| Graphy | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| Guidewheel | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| Gumroad | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| Harmony | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| HashiCorp | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| Heap | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| Helium 10 | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| Hopin | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| Hubspot | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| Hugging Face | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| InfluxData | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| insightsoftware | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| JupiterOne | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| Kilo Code | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| Kit | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| Kraken | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| League | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| Loft | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| Loom | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| Loops | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| Marqeta | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| Mastodon | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| Medallia | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| Mr Yum | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| Mural | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| NCX | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| Newfront | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| Numbrs | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| Obsidian | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| OKcoin | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| OnDeck | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| Oracle | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| Osano | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| Pacvue | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| Pagos | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| Parabol | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| Posit | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| ProjectDiscovery | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| Protocol Labs | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| Prufrock Ventures | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| Quora | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| Red Hat | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| Refersion | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| Relativity Space | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| Rithum | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| Robin Healthcare | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| Root Insurance | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| RootstockLabs | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| SafetyWing | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| Shogun | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| Shopify | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| Shopmonkey | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| Skillshare | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| Slack | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| Spruce | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| Superhuman | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| Superhuman Docs | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| SUSE | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| Syndicate | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| Sysdig | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| Tabular | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| Teamshares | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| Teleport | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| The Browser Company | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| Toggl | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| Trust Machines | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| Turso | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| Varsity Tutors | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| Veeva Systems | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| VSCO | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| Wave | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| Whatnot | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| Wonolo | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| Xapo | Unresolved CSV entry; board slug/provider intentionally not guessed. |
| Yelp | Unresolved CSV entry; board slug/provider intentionally not guessed. |

## Validation and deployment

- Regression tests require every CSV name to remain represented and every enabled entry to carry source and notes.
- `cargo test --all-targets` uses a disposable PostgreSQL database; do not run database tests against production.
- Registry is compiled via `include_str!`: a rebuild/deploy and authenticated sync are required before these additions appear in production. This pass does not deploy or trigger production sync.

## Next discovery pass

- Resolve the disabled names above using actual company careers links rather than bulk guessing slugs.
- Verify Workday tenant/shard/site for large employers and preserve pagination/detail fetching.
- Recheck existing enabled mappings for freshness separately; enabled count is not operational-health count.
- Expand beyond the CSV using developer-tool, infrastructure, and remote-first company careers pages, deduplicating employer names and provider/board identities before enabling.

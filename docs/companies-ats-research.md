# Initial ATS coverage research

Checked 2026-10-03. Candidate missing companies were researched against official careers pages/search results; public feed URLs below were then requested directly. HTTP 200 confirms endpoint accessibility, not necessarily current postings (Ashby `deel` currently returns an empty jobs array).

| CSV company | ATS | Board slug | Direct public feed tested | Notes |
|---|---|---|---|---|
| AG1 | Greenhouse | `ag1` | `https://boards-api.greenhouse.io/v1/boards/ag1/jobs` | 200; official board `https://job-boards.greenhouse.io/ag1` |
| Automattic | Greenhouse | `automatticcareers` | `https://boards-api.greenhouse.io/v1/boards/automatticcareers/jobs` | 200; official careers links to this board |
| Buffer | Ashby | `buffer` | `https://api.ashbyhq.com/posting-api/job-board/buffer` | 200; official careers page routes to Ashby |
| Certa | Ashby | `certa` | `https://api.ashbyhq.com/posting-api/job-board/certa` | 200 |
| Chili Piper | Ashby | `chilipiper` | `https://api.ashbyhq.com/posting-api/job-board/chilipiper` | 200; use current Ashby board, not legacy Greenhouse/SmartRecruiters references |
| DNSFilter | Greenhouse | `dnsfilter` | `https://boards-api.greenhouse.io/v1/boards/dnsfilter/jobs` | 200 |
| Doist | Workable | `doist` | `https://apply.workable.com/api/v1/widget/accounts/doist?details=false` | 200 (board reports no current openings in search results) |
| Jeeves | Lever | `tryjeeves` | `https://api.lever.co/v0/postings/tryjeeves?mode=json` | 200 |
| Lokalise | Greenhouse | `lokalise` | `https://boards-api.greenhouse.io/v1/boards/lokalise/jobs` | 200 |
| Midjourney | Ashby | `midjourney` | `https://api.ashbyhq.com/posting-api/job-board/midjourney` | 200; official careers page points to Ashby |
| MoonPay | Lever | `moonpay` | `https://api.lever.co/v0/postings/moonpay?mode=json` | 200 |
| Sourcegraph | Greenhouse | `sourcegraph91` | `https://boards-api.greenhouse.io/v1/boards/sourcegraph91/jobs` | 200 |
| GrowthX AI | Ashby | `GrowthX AI` (URL-encoded as `GrowthX%20AI`) | `https://api.ashbyhq.com/posting-api/job-board/GrowthX%20AI` | 200; 2 jobs returned when checked; user supplied official job URL confirming board name/listing ID |
| Deel | Ashby (careers evidence) | unverified | tested `deel`: 200 but empty jobs | Application links show `ashby_jid`; endpoint slug does not establish valid board. Needs further verification. |

Sources (official/careers evidence): https://www.ag1.com/careers/ or https://job-boards.greenhouse.io/ag1 ; https://automattic.com/work-with-us/jobs/ ; https://buffer.com/journey ; https://www.chilipiper.com/careers ; https://dnsfilter.com/careers ; https://apply.workable.com/doist/ ; https://www.tryjeeves.com/careers ; https://lokalise.com/careers/ ; https://www.midjourney.com/careers ; https://www.moonpay.com/careers ; https://sourcegraph.com/jobs ; https://growthx.ai/careers ; https://jobs.ashbyhq.com/GrowthX%20AI/1d32b584-d343-49aa-8e62-3dc52fc59ebb ; https://www.deel.com/careers/.

These mappings are ready to encode as company/provider/board entries once the registry schema and adapters are implemented. Public HTTP 200 checks were made against the ATS APIs, but production integration should still parse and test representative fixture payloads. ATS/API does not guarantee the employer is hiring or that a role is open to a particular location.

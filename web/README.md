# Job tracker UI

Minimal semantic HTML with native browser styles—no CSS framework or stylesheet. `app.js` calls the planned Catalyst API routes (`GET/POST /jobs`, `PATCH/DELETE /jobs/:id`).

Before connecting it, set `window.JOB_API_BASE` to the deployed Catalyst API Gateway URL in `index.html`. The API and Data Store are not deployed yet. Serve this folder locally with any static HTTP server; opening `index.html` directly may restrict browser requests.

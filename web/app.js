// Set window.JOB_API_BASE to the deployed Catalyst API Gateway URL before this script.
// Example shape: https://<project>.catalystserverless.com/<route>
const API_BASE = window.JOB_API_BASE;
const form = document.querySelector("#job-form");
const list = document.querySelector("#jobs-list");
const message = document.querySelector("#message");
const filter = document.querySelector("#status-filter");
const template = document.querySelector("#job-template");
let jobs = [];

async function request(path, options = {}) {
  if (!API_BASE) throw new Error("Set JOB_API_BASE to connect the Catalyst API.");
  const response = await fetch(`${API_BASE}${path}`, {
    ...options,
    headers: { "Content-Type": "application/json", ...options.headers },
  });
  if (!response.ok) throw new Error(`API request failed (${response.status}).`);
  return response.status === 204 ? null : response.json();
}

function render() {
  list.replaceChildren();
  const visible = jobs.filter(job => filter.value === "all" || job.status === filter.value);
  for (const job of visible) {
    const row = template.content.firstElementChild.cloneNode(true);
    row.querySelector(".job-title").textContent = job.title;
    row.querySelector(".job-company").textContent = job.company;
    row.querySelector(".job-location").textContent = job.location || "Location not specified";
    const link = row.querySelector(".job-link");
    link.href = job.source_url;
    const status = row.querySelector(".job-status");
    status.value = job.status;
    status.addEventListener("change", async () => {
      try {
        await request(`/jobs/${encodeURIComponent(job.id)}`, {
          method: "PATCH", body: JSON.stringify({ status: status.value }),
        });
        job.status = status.value;
        render();
        message.textContent = "Job updated.";
      } catch (error) { message.textContent = error.message; }
    });
    row.querySelector(".delete-job").addEventListener("click", async () => {
      try {
        await request(`/jobs/${encodeURIComponent(job.id)}`, { method: "DELETE" });
        jobs = jobs.filter(item => item.id !== job.id);
        render();
        message.textContent = "Job deleted.";
      } catch (error) { message.textContent = error.message; }
    });
    list.append(row);
  }
  if (visible.length === 0 && jobs.length > 0) message.textContent = "No jobs match this status.";
  else if (visible.length === 0) message.textContent = "No saved jobs yet.";
  else message.textContent = `${visible.length} job${visible.length === 1 ? "" : "s"}.`;
}

async function loadJobs() {
  message.textContent = "Loading jobs…";
  try {
    const result = await request("/jobs");
    jobs = result.jobs;
    render();
  } catch (error) { message.textContent = error.message; }
}

form.addEventListener("submit", async event => {
  event.preventDefault();
  const data = Object.fromEntries(new FormData(form));
  data.status = "saved";
  try {
    const result = await request("/jobs", { method: "POST", body: JSON.stringify(data) });
    jobs.unshift(result.job);
    form.reset();
    render();
    message.textContent = "Job saved.";
  } catch (error) { message.textContent = error.message; }
});

filter.addEventListener("change", render);
loadJobs();

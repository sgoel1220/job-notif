# Job notifications MVP: proposed storage and API contract

**Status: proposal only.** No Catalyst project IDs, environment IDs, or live Data Store schema have been provided. This document does not claim that a Catalyst table or function has been created or deployed. Confirm field types, identifiers, and deployment settings against the target Catalyst environment before implementation.

## Proposed Data Store table: `Jobs`

One row represents a saved job listing. Catalyst Data Store assigns a row identifier (`ROWID`); treat it as an opaque string in the API. The table name and fields below are proposed and must be verified against Catalyst naming and type constraints.

| Proposed field | Proposed type | Required | Meaning |
| --- | --- | --- | --- |
| `title` | Text | Yes | Job title. |
| `company` | Text | Yes | Employer or organization name. |
| `location` | Text | No | Human-readable location; may be remote. |
| `source_url` | Text | Yes | Canonical URL for the listing. |
| `status` | Text | Yes | MVP state: `saved`, `applied`, or `dismissed`. Default `saved`. |
| `created_time` | Catalyst-managed creation timestamp | Yes | Set on row creation; clients should not choose or overwrite it. |

The Catalyst-managed creation timestamp may be exposed as a system field rather than a custom column. Confirm the actual Data Store timestamp field name and representation before mapping it to `created_time`. Do not add a custom timestamp column until that is verified.

Suggested constraints: validate non-empty `title`, `company`, and `source_url`; validate `source_url` as an absolute HTTP(S) URL; reject statuses outside the stated set. Uniqueness by `source_url` is a product decision and is not assumed here.

## Supported CRUD behaviors

Expose these behaviors through a TypeScript Catalyst Function. The route names are illustrative; adapt them to the selected Catalyst function type and API Gateway routing configuration.

| Behavior | Request | Success response | Notes |
| --- | --- | --- | --- |
| List jobs | `GET /jobs` | `200` with `{ jobs: Job[] }` | Return an empty array when no rows exist. Pagination/filtering can be added later. |
| Read one | `GET /jobs/:id` | `200` with `{ job: Job }` | `404` if the row identifier does not exist. |
| Create | `POST /jobs` with `CreateJobRequest` | `201` with `{ job: Job }` | Server assigns `id` and `created_time`; status defaults to `saved`. |
| Update | `PATCH /jobs/:id` with `UpdateJobRequest` | `200` with `{ job: Job }` | Partial update of supplied mutable fields only; `404` if absent. |
| Delete | `DELETE /jobs/:id` | `204` with no body | `404` if absent. |

For malformed JSON or invalid fields, return `400` with a stable error envelope. For unexpected storage or server failures, return `500` without exposing internal error details. Apply the project's chosen authentication and authorization policy before exposing these operations; none is assumed by this proposal.

## TypeScript contract

This is a framework-independent contract suitable for a TypeScript Catalyst Function. The Catalyst SDK adapter should translate between these shapes and actual Data Store rows, including the confirmed Catalyst row ID and timestamp fields.

```ts
export type JobStatus = "saved" | "applied" | "dismissed";

/** API representation; id and created_time are assigned by the server. */
export interface Job {
  id: string;
  title: string;
  company: string;
  location: string | null;
  source_url: string;
  status: JobStatus;
  created_time: string; // ISO 8601 timestamp
}

export interface CreateJobRequest {
  title: string;
  company: string;
  location?: string | null;
  source_url: string;
  status?: JobStatus; // defaults to "saved"
}

export type UpdateJobRequest = Partial<CreateJobRequest>;

export interface ListJobsResponse {
  jobs: Job[];
}

export interface GetJobResponse {
  job: Job;
}

export interface CreateJobResponse {
  job: Job;
}

export interface UpdateJobResponse {
  job: Job;
}

export interface ApiError {
  error: {
    code: "BAD_REQUEST" | "NOT_FOUND" | "INTERNAL_ERROR";
    message: string;
  };
}
```

### Contract rules

- `id` is the API's opaque string representation of the Catalyst row identifier; it is not a caller-provided table field.
- `created_time` is serialized as an ISO 8601 string in API responses and is server-managed.
- `location` is normalized to `null` when absent in responses; callers may omit it or send `null` on create/update.
- `PATCH` must reject an empty update and must not accept `id` or `created_time` as writable fields.
- A successful delete returns `204 No Content` (no JSON body).
- Map storage errors to the stable error envelope; do not leak raw Catalyst SDK errors to callers.

# NextRequest Platform API - Complete Technical Reference Manual

This specification maps the endpoint layouts, payload field keys, query strings, and pagination
behavior for CivicPlus NextRequest portals, and records what this client actually sends.

NextRequest publishes no stable contract, so the client does not hard-code an endpoint: it probes
candidate pagination parameter layouts (`page`/`per_page`, then `page`/`page_size`) during portal
discovery and persists whichever the portal accepts as `PortalApiLayout`
(`ingestion/client.rs`). The endpoint below is the shape discovery looks for, not a constant.

---

## 1. Public Requests Listing Matrix

Retrieves paginated arrays of public records requests and their associated metadata envelopes.

* **HTTP Method:** `GET`
* **Target Endpoint URL:** `https://<agency-subdomain>.nextrequest.com/client/request_archives`
  (discovered per portal and stored as `PortalApiLayout::requests_endpoint`; the parameter names
  are discovered alongside it and are not assumed)
* **Content Transport Type:** `application/json`

### Query Parameter Specifications

| Parameter | Data Type | Default Value | Max Value Range | Functional Description |
| :--- | :--- | :--- | :--- | :--- |
| `page` | Unsigned Integer | `1` | None | Identifies the targeted paginated slice window to return. |
| `per_page` | Unsigned Integer | `50` | `100` | Records per page. **This client** defaults to `50` (`commands/ingest.rs`) and rejects anything outside `1..=100` before sending (`ingestion/client.rs`). No client-side truncation exists; an out-of-range value is an error, not a clamp. The parameter may instead be named `page_size` depending on what discovery found. |
| `status` | String Option | None | `open`, `closed` | Accepted by the portal. **This client never sends it** — only the page and page-size parameters are attached. Status filtering happens locally, after ingestion. |

### Headers this client sends
```text
User-Agent: OnRecord-Studio/1.0 public-records-audit-client
Accept: application/json, text/html;q=0.5, */*;q=0.1
```
The client identifies itself honestly rather than impersonating a browser. Redirects are not
followed automatically (`redirect::Policy::none()`); each hop is re-validated by the caller.

### Complete Response Payload Schema (`HTTP 200 OK`)
```json
{
  "requests": [
    {
      "id": 481029,
      "rpid": "NR-2026-9912A",
      "title": "Encampment Enforcement Log - Sector 4",
      "description": "Public records request seeking multi-department operations coordination transcripts, clean up task assignments, and localized dispatch tracking records.",
      "status": "closed",
      "current_status_id": 14,
      "created_at": "2026-02-14T08:15:30.000Z",
      "updated_at": "2026-02-16T17:45:00.000Z",
      "closed_at": "2026-02-16T17:45:00.000Z",
      "due_date": "2026-02-24T17:00:00.000Z",
      "agency": {
        "id": 882,
        "name": "Environmental Services Department",
        "department_name": "Code Compliance Operations Division"
      },
      "category": {
        "id": 41,
        "name": "Abatement Log Entries"
      },
      "documents": [
        {
          "id": 901245,
          "file_name": "abatement_coordination_final.pdf",
          "file_size": 2491028,
          "description": "Released public safety operations coordination ledger data sheet.",
          "content_type": "application/pdf",
          "download_url": "https://nextrequest.com",
          "created_at": "2026-02-16T17:40:00.000Z",
          "updated_at": "2026-02-16T17:40:00.000Z",
          "visibility": "public"
        }
      ]
    }
  ],
  "meta": {
    "current_page": 1,
    "next_page": 2,
    "prev_page": null,
    "total_pages": 48,
    "total_count": 960
  }
}
```

---

## 2. Binary Document Downloader Pathway

Downloads raw file payloads associated with a target request ID.

* **HTTP Method:** `GET`
* **Target Endpoint URL:** `https://<agency-subdomain>.nextrequest.com/documents/<document_id>/download`
  (document URLs are taken from the listing payload rather than constructed; every URL is run
  through `validate_remote_url` before it is fetched)
* **Content Transport Type:** `application/octet-stream` (or matches target mime, e.g., `application/pdf`)

### Path Routing Bounds

| Route Segment | Type | Validation Rule | Description |
| :--- | :--- | :--- | :--- |
| `<document_id>` | Unsigned Long | Must match an active `id` field inside the parent request array. | Target reference key identifying the file payload to extract. |

### Expected Gateway Response Transport States

| Status Code | Reason/Type | Response Content | Ingestion Core Error Mitigation Strategy |
| :--- | :--- | :--- | :--- |
| `200 OK` | File Stream | Binary Stream | Stream chunks are piped directly to disk via `tokio::io::AsyncWriteExt`. |
| `429 Too Many Requests` | Throttling | JSON Error Message | Triggers the token bucket system to sleep, executing an exponential backoff retry loop. |
| `403 Forbidden` | Access Blocked | HTML / Shield page | Logs a pipeline failure flag (`TaskFailed`) due to staff-only visibility restrictions. |
| `404 Not Found` | Broken Link | JSON / Empty | Skips file execution, flagging the document entry database column as `failed`. |

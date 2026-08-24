# NextRequest Platform API - Complete Technical Reference Manual

This specification maps the internal endpoint layouts, payload field keys, query strings, and pagination behaviors for CivicPlus NextRequest portal environments.

---

## 1. Public Requests Listing Matrix

Retrieves paginated arrays of public records requests and their associated metadata envelopes.

* **HTTP Method:** `GET`
* **Target Endpoint URL:** `https://<agency-subdomain>://`
* **Content Transport Type:** `application/json`

### Query Parameter Specifications

| Parameter | Data Type | Default Value | Max Value Range | Functional Description |
| :--- | :--- | :--- | :--- | :--- |
| `page` | Unsigned Integer | `1` | None | Identifies the targeted paginated slice window to return. |
| `per_page` | Unsigned Integer | `20` | `50` | Dictates individual bulk record counts per payload segment. Values exceeding `50` are automatically truncated by the server gateway. |
| `status` | String Option | None | `open`, `closed` | Optional filter to isolate records based on their lifecycle status. |

### Header Requirements
```text
Host: <agency-subdomain>.nextrequest.com
Accept: application/json, text/plain, */*
Connection: keep-alive
```

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
* **Target Endpoint URL:** `https://<agency-subdomain>://<document_id>/download`
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

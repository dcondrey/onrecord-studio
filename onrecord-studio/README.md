# OnRecord Studio — San Diego

A desktop investigation workspace focused on San Diego homelessness, encampment abatement, public bathrooms, police spending, and contract performance.

The home screen presents autonomous research dossiers: specific findings with quoted evidence, competing explanations, next tests, tailored public-records requests, and dated Council follow-through. While the desktop app is open, the native service collects primary records, develops research questions, runs document searches and SQL comparisons, and critically reviews its findings. Evidence gaps can trigger additional primary-source discovery.

Set `OPENROUTER_API_KEY` or `OPENAI_API_KEY` in your environment or `~/.env`, or choose an installed local Ollama model in **Research settings**. Defaults allow 24 model calls per day; Hosted model and search usage is billed to your key. OpenRouter is selected initially when its key is available. Eligible processed public downloads are released automatically while source URLs, hashes, searchable derivatives, and compressed dossiers are retained. Local imports and incomplete extractions retain their original files.

Use **Analysis** to select an acquired table or import a local CSV, TSV, JSON, GeoJSON, or spreadsheet. Inspect source snapshots, review rejected rows, and compare City budgets with actuals by fiscal year, fund, and account. SQL preserves exact amounts and identifiers and exposes missing counterparts. [Regional ingestion and analysis](docs/regional-data-ingestion.md) describes 118 regional and housing source routes, supported formats, verification, and limits, including local-export workflows for FTP/SFTP and restricted portals.

## Installing and using

OnRecord Studio ships for macOS 13 or later as a Mac App Store app and as a notarized direct
download. The [user guide](docs/user-guide.md) covers installation, first launch, the investigation
workflow, privacy, data location and backup, updates, and troubleshooting. The same material is in
the app under Help (Cmd+Shift+/). Scanned documents are only searchable when
[Tesseract and Poppler](docs/user-guide.md#optional-ocr) are installed; everything else is built in.

## Developing

Run the desktop app from this package:

```sh
npm install
npm run tauri -- dev
```

Development requires Rust and the Tauri macOS build prerequisites. `npm run dev` starts only the frontend preview; acquisition, storage, search, and extraction require the native application. Release builds are described in [release.md](docs/release.md) (Mac App Store) and [release-direct.md](docs/release-direct.md) (Developer ID).

[San Diego scope and validation](docs/san-diego-focus.md) defines the current product direction. [The roadmap](docs/ROADMAP.md) preserves the implementation history, and [query-language architecture](docs/query-language.md) documents search behavior.

The investigation workbench connects persistent cases, request production tracking, exact financial reconciliation, source-change and service-access monitoring, evidence review, portable case collaboration, and usefulness assessments. See the [twenty-item roadmap](docs/investigative-product-roadmap.md) and [validation record](docs/validation/investigative-workbench-2026-09-26.md).

# OnRecord Studio — San Diego

A local desktop workspace for researching public records and datasets across the San Diego region and preparing evidence-backed public comment. The research desk includes City, County, regional, environmental, education, election, housing, and research sources alongside the Commission on Police Practices and SDPD collections.

Choose a maintained San Diego source, add its published documents, search and review the underlying text, then save a lead or excerpt to a briefing. The home screen explains each source's coverage; importing a calendar does not imply that the entire historical archive is present.

Use **Analysis** to select an acquired table or import a local CSV, TSV, JSON, GeoJSON, or spreadsheet. Inspect source snapshots, review rejected rows, and compare City budgets with actuals by fiscal year, fund, and account. SQL preserves exact amounts and identifiers and exposes missing counterparts. [Regional ingestion and analysis](docs/regional-data-ingestion.md) describes 118 regional and housing source routes, supported formats, verification, and limits, including local-export workflows for FTP/SFTP and restricted portals.

Run the desktop app from this package:

```sh
npm install
npm run tauri -- dev
```

The app requires Rust and the Tauri macOS build prerequisites. `npm run dev` starts only the frontend preview; acquisition, storage, search, and extraction require the native application.

[San Diego scope and validation](docs/san-diego-focus.md) defines the current product direction. [The roadmap](docs/ROADMAP.md) preserves the implementation history, and [query-language architecture](docs/query-language.md) documents search behavior.

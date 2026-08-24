# OnRecord Studio - Forensic Analytical Methodologies & Calculation Specifications

This document defines the mathematical models, statistical calculations, and Natural Language Processing (NLP) routines executed inside the DuckDB and SQLite layers of `onrecord-studio`.

---

## 1. Chronological Timeline Anomaly Detection

The native Rust backend analyzes public record timestamp histories to detect structural signs of data manipulation or administrative backlog trimming.

### A. Chronological Discrepancy Scans
The system compares creation and modification timestamps to identify logical sequence errors:
\[\Delta t_{\text{skew}} = t_{\text{updated\_at}} - t_{\text{created\_at}}\]
\[\text{Condition Flag} = \Delta t_{\text{skew}} < 0 \implies \text{UPDATE\_BEFORE\_CREATION}\]

\[\Delta t_{\text{lifespan}} = t_{\text{closed\_at}} - t_{\text{created\_at}}\]
\[\text{Condition Flag} = \Delta t_{\text{lifespan}} < 0 \implies \text{CLOSE\_BEFORE\_CREATION}\]

### B. Arbitrary Closure Index (Fast-Drop Filtering)
Agencies may quickly dismiss incoming public interest requests to artificially lower their open-ticket metrics. The system flags this behavior by measuring response times against a strict processing window:
\[\text{FastDrop} = (\Delta t_{\text{lifespan}} \le 300\text{ seconds}) \land (\text{Status} = \text{"Closed"})\]
* **Triage Routing:** Triggering this flag marks the record as `SUSPICIOUSLY_FAST_CLOSURE` and ranks it higher in the inspector's manual audit list.

---

## 2. Multi-Variable Departmental Outlier Detection (OLAP Core)

The analytical engine uses DuckDB to calculate citywide performance metrics, identifying specific agencies with statistically abnormal delays.

```text
[ Relational SQLite Store ] ---> [ Columnar In-Memory Processing Partition (DuckDB) ]
                                                      |
                                                      v
                                  ( Group-By Aggregations: Average Processing Days )
                                                      |
                                                      v
                                  ( Standard Deviation Windows Calculation (\sigma) )
                                                      |
                                                      v
                                [ Flags Outliers Exceeding Legal Threshold Bounds ]
```

### A. Global Variance Formulations
The system computes the global average response latency (μ) and standard deviation (σ) across all processed cases:
\[\mu_{\text{global}} = \frac{1}{N}\sum_{i=1}^{N}\Delta t_{\text{lifespan}, i}\]

\[\sigma_{\text{global}} = \sqrt{\frac{1}{N}\sum_{i=1}^{N}(\Delta t_{\text{lifespan}, i} - \mu_{\text{global}})^2}\]

### B. Departmental Z-Score Ranking
The system monitors departmental performance by calculating Z-scores relative to citywide processing averages:
\[Z_{\text{agency}} = \frac{\mu_{\text{agency}} - \mu_{\text{global}}}{\sigma_{\text{global}}}\]
* **Statistical Classifications:**
  * Z ≥ +1.0: Administrative Bottleneck.
  * Z ≥ +2.0: Critical Non-Compliance Outlier (High systemic liability risk).

---

## 3. High-Density Text Redaction Analysis

When files are parsed, the system measures the density of redacted text blocks relative to standard content to identify hidden information dumps.

\[\text{Redaction Ratio} (R_d) = \frac{\text{Count of Redacted Pattern Placeholders}}{\text{Total Parsed Character Array Size}}\]
* **Pattern Configurations:** Scans for regular expression strings used by government redaction software (e.g., `(?i)\[redacted\]`, `(?i)\[withheld\s+under\s+exemption\]`, `\u2588{3,}` [solid black blocks]).
* **Triage Routing:** Files where \(R_d > 0.45\) are flagged with `HEAVY_REDACTION_DENSITY`, pointing investigators to files that may require legal review or formal appeal challenge processes.

---

## 4. Local NLP Keyword Clustering & Density Maps

To isolate records tied to systemic homelessness or housing rights violations, the engine processes unformatted description text through a case-insensitive keyword filter.

### A. Word-Stem Boundary Target Matrix
```text
\b(sweep(s|ing|ed)?|encampment(s)?|clean(ing|\s+up)?|evict(ion|s|ed)?|displace(ment|d)?|property\s+storage)\b
```

### B. Density Calculation
$$\text{Keyword Density} (D_k) = \frac{\text{Total Count of Keyword Matches}}{\text{Total Count of Words in Case Description}} \times 100$$
* **Application:** High $D_k$ scores filter out generic municipal maintenance requests, isolating target enforcement events across the dataset.

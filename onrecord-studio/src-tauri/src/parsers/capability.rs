use super::{DocumentFormat, ExtractionRoute};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParserCostClass {
    Lightweight,
    CpuBound,
    OcrBound,
    ContainerExpansion,
    Deferred,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParserCapability {
    pub format: DocumentFormat,
    pub route: ExtractionRoute,
    pub maximum_source_bytes: u64,
    /// Conservative source-to-working-set ratio used by the queue's shared memory governor.
    pub working_set_multiplier: u32,
    pub cost_class: ParserCostClass,
    pub residual_evidence_audit: bool,
    /// External parser code is never loaded into the process. Unsupported formats are reported as
    /// deferred and may be converted out of process by an operator before local import.
    pub trusted_core_only: bool,
}

pub const fn parser_capability(format: DocumentFormat) -> ParserCapability {
    let route = format.extraction_route();
    let (working_set_multiplier, cost_class) = match route {
        ExtractionRoute::PlainText | ExtractionRoute::RichText | ExtractionRoute::SqlDump => {
            (2, ParserCostClass::Lightweight)
        }
        ExtractionRoute::ImageOcr => (4, ParserCostClass::OcrBound),
        ExtractionRoute::Container(_) => (3, ParserCostClass::ContainerExpansion),
        ExtractionRoute::ToolRequired(_)
        | ExtractionRoute::KeyRequired
        | ExtractionRoute::ImageWithoutText
        | ExtractionRoute::Unknown => (1, ParserCostClass::Deferred),
        _ => (3, ParserCostClass::CpuBound),
    };
    let residual_evidence_audit = matches!(
        format,
        DocumentFormat::Pdf
            | DocumentFormat::Image
            | DocumentFormat::Zip
            | DocumentFormat::WordPackage
            | DocumentFormat::PresentationPackage
            | DocumentFormat::OpenDocumentText
            | DocumentFormat::OpenDocumentPresentation
    );
    ParserCapability {
        format,
        route,
        maximum_source_bytes: match format {
            DocumentFormat::OutlookStore => super::pst::MAX_PST_BYTES,
            _ => 512 * 1024 * 1024,
        },
        working_set_multiplier,
        cost_class,
        residual_evidence_audit,
        trusted_core_only: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expensive_routes_have_distinct_governor_profiles() {
        assert_eq!(
            parser_capability(DocumentFormat::Image).cost_class,
            ParserCostClass::OcrBound
        );
        assert!(parser_capability(DocumentFormat::Image).working_set_multiplier > 2);
        assert_eq!(
            parser_capability(DocumentFormat::Zip).cost_class,
            ParserCostClass::ContainerExpansion
        );
        assert_eq!(
            parser_capability(DocumentFormat::Access).cost_class,
            ParserCostClass::Deferred
        );
        assert!(parser_capability(DocumentFormat::Pdf).residual_evidence_audit);
        assert!(parser_capability(DocumentFormat::Pdf).trusted_core_only);
    }
}

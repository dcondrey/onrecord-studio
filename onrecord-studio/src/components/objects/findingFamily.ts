import type { FindingKind } from '../../types/findings';

/** The lead families a finding is filed under. `Outlier` belongs to statistical agency rows only. */
export type LeadFamily = 'Records request' | 'Spending' | 'Contradiction' | 'Pattern' | 'Data gap' | 'Outlier';

/** Exhaustive by construction: a new Rust kind fails `tsc` here until it is filed. */
export function findingFamily(kind: FindingKind): Exclude<LeadFamily, 'Outlier'> {
  switch (kind) {
    case 'HELD_BACK_RESPONSE':
    case 'BOILERPLATE_DENIAL':
    case 'JUST_IN_TIME_COMPLIANCE':
    case 'EXTENSION_ABUSE':
    case 'EVASION_PARAPHRASED_DENIALS':
    case 'EVASION_ROUND_NUMBER_THRESHOLDS':
    case 'EVASION_REDACTION_AT_LIMIT':
    case 'EVASION_LATE_ARRIVING_RECORDS':
    case 'REDACTION_CANDIDATE_INFERRED':
    case 'REDACTION_INCONSISTENT_ACROSS_RELEASES':
    case 'POLICY_VIOLATION_DETECTED':
    case 'PRACTICE_DEPARTURE_COUNTED':
    case 'PRACTICE_DEPARTURE_STATED':
    case 'ROLLING_PRODUCTION':
    case 'RELEASE_TIMING_CONCENTRATION':
      return 'Records request';
    case 'CONTRACT_CEILING_EXCEEDED':
    case 'PAYMENT_OUTSIDE_CONTRACT_TERM':
    case 'PAYMENT_WITHOUT_CONTRACT_RECORD':
    case 'APPROPRIATION_EXCEEDED':
    case 'RESTRICTED_FUND_OFF_PROGRAM':
    case 'APPROVAL_THRESHOLD_EXCEEDED':
    case 'APPROVAL_RECORD_ABSENT':
    case 'SPLIT_PURCHASE_PATTERN':
    case 'SPLIT_PURCHASE_PROHIBITED_BY_POLICY':
    case 'DEPARTMENT_SPENDING_YEAR_OVER_YEAR':
    case 'OVERTIME_ABOVE_ADOPTED_LINE':
    case 'FUND_ACTUAL_ABOVE_ADOPTED':
    case 'SPENDING_RECORD_LEAD':
      return 'Spending';
    case 'METADATA_CONTRADICTS_CONTENT':
    case 'CONTRADICTORY_STATEMENT':
    case 'CLAIM_CONTRADICTS_DATA':
    case 'NO_RECORDS_CONTRADICTED_BY_RELEASE':
    case 'REGENERATED_RESPONSE':
      return 'Contradiction';
    case 'CROSS_CORPUS_SHARED_LANGUAGE':
    case 'CROSS_CORPUS_TEMPORAL_SHIFT':
    case 'CROSS_CORPUS_ORPHAN_TOPIC':
    case 'CROSS_CORPUS_REUSED_DENIAL_GROUNDS':
    case 'CORRELATED_FINDINGS':
    case 'DIFFERENTIAL_TREATMENT':
    case 'ENCAMPMENT_NOTICE_SHORTFALL':
    case 'ENCAMPMENT_NOTICE_STATED_SHORTFALL':
    case 'SERVICE_REPORT_LEAD':
    case 'POLICE_STOP_LEAD':
    case 'POLICE_CALL_LEAD':
    case 'ABATEMENT_LOG_LEAD':
    case 'CHARGE_LOG_LEAD':
    case 'DISCLOSURE_FILING_LEAD':
    case 'CAPACITY_COMPARISON_LEAD':
    case 'PARKING_CITATION_LEAD':
    case 'HOUSING_APPROVAL_LEAD':
      return 'Pattern';
    case 'ABSENCE_OF_RECORDS':
    case 'EMAIL_THREAD_GAP':
      return 'Data gap';
    default: {
      const unreachable: never = kind;
      return unreachable;
    }
  }
}

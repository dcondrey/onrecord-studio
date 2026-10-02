export interface FacilityBoundingBox {
  minLatitude: number;
  minLongitude: number;
  maxLatitude: number;
  maxLongitude: number;
}

export interface FacilityArea {
  name: string;
  boundingBox: FacilityBoundingBox | null;
  facilityIds: string[];
}

export interface FacilityCoverageMapping {
  snapshotId: string;
  nameColumn: string;
  idColumn: string | null;
  addressColumn: string | null;
  latitudeColumn: string | null;
  longitudeColumn: string | null;
  hoursColumn: string | null;
  categoryColumn: string | null;
  restroomCategories: string[];
  areas: FacilityArea[];
}

export type FacilityHoursStatus = 'RECORDED' | 'UNINTERPRETABLE' | 'NOT_RECORDED';

export interface FacilityOpenInterval {
  startMinute: number;
  endMinute: number;
}

export interface FacilityCoverageFacility {
  tableRow: number;
  facilityId: string | null;
  name: string | null;
  address: string | null;
  hoursText: string | null;
  hoursStatus: FacilityHoursStatus;
  openIntervals: FacilityOpenInterval[];
}

export interface FacilityAreaCoverage {
  areaName: string;
  boundary: string;
  publicRestrooms: number;
  withRecordedHours: number;
  hoursUninterpretable: number;
  hoursNotRecorded: number;
  openMinutesCovered: number | null;
  minutesWithNoneOpen: number | null;
  unmatchedFacilityIds: string[];
  /** Listed ids present in the import but outside the mapped restroom categories. */
  excludedFacilityIds: string[];
  facilities: FacilityCoverageFacility[];
  facilitiesOmitted: number;
}

export interface FacilityCoverageReport {
  mapping: FacilityCoverageMapping;
  datasetName: string;
  tableName: string;
  sourceUri: string | null;
  sourceSha256: string;
  totalRows: number;
  publicRestroomRows: number;
  locationNotRecorded: number | null;
  areas: FacilityAreaCoverage[];
  limitations: string[];
}

export interface FacilityCoverageWorkspace {
  reports: FacilityCoverageReport[];
  warnings: string[];
}

export interface TrustedContributor {
  keyId: string;
  label: string;
  trustedAt: string;
}

export interface LocalContributorIdentity {
  keyId: string;
}

export interface SharedFolderPacket {
  fileName: string;
  bytes: number;
  modifiedAt: string | null;
  /** Over the transport limit: listed so it is not silently missing, but it cannot be read. */
  oversized: boolean;
}

export interface SharedFolderListing {
  packets: SharedFolderPacket[];
  /** More packets exist than are listed; the listed ones are the newest. */
  truncated: boolean;
}

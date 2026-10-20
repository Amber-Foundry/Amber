-- Migration: 0013_privacy_3tier_consolidation.sql
-- Streamline privacy model from 4 tiers ('open', 'local_only', 'locked', 'redacted')
-- down to 3 tiers ('open', 'local_only', 'redacted').
-- Remap all existing 'locked' privacy_tier records in vaults, sub_vaults, nodes,
-- and privacy_overrides to 'redacted' to guarantee zero data exposure / privacy regression.

-- 1. Remap vaults table
UPDATE vaults
SET privacy_tier = 'redacted'
WHERE privacy_tier = 'locked';

-- 2. Remap sub_vaults table (legacy table synchronized via triggers)
UPDATE sub_vaults
SET privacy_tier = 'redacted'
WHERE privacy_tier = 'locked';

-- 3. Remap nodes table
UPDATE nodes
SET privacy_tier = 'redacted'
WHERE privacy_tier = 'locked';

-- 4. Remap privacy_overrides table (if present)
UPDATE privacy_overrides
SET privacy_tier = 'redacted'
WHERE privacy_tier = 'locked';

-- Software engineering is orthogonal to seniority/internship categories.
CREATE FUNCTION job_is_software_engineering(job_title TEXT)
RETURNS BOOLEAN LANGUAGE SQL IMMUTABLE PARALLEL SAFE AS $$
    SELECT job_title ~* '\m(software[[:space:]-]+(development[[:space:]-]+)?(engineer(ing)?|developer)|sde([[:space:]-]*[0-9]+)?|((front|back)[[:space:]-]*end|full[[:space:]-]*stack|mobile|ios|android|embedded)[[:space:]-]+(software[[:space:]-]+)?(engineer(ing)?|developer))\M'
$$;

ALTER TABLE job_postings
    ADD COLUMN is_software_engineering BOOLEAN GENERATED ALWAYS AS (job_is_software_engineering(title)) STORED;
CREATE INDEX idx_job_postings_active_software_engineering ON job_postings(id)
    WHERE is_active = TRUE AND is_software_engineering = TRUE;

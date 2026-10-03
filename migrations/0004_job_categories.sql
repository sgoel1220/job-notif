-- Deterministic mappings: unknown levels/countries are not inferred from descriptions.
-- Stored generated columns backfill existing rows and recompute on every insert/update.
CREATE FUNCTION job_role_category(job_title TEXT, job_employment TEXT)
RETURNS TEXT LANGUAGE SQL IMMUTABLE PARALLEL SAFE AS $$
    SELECT CASE
        WHEN job_title ~* '\m(intern|interns|internship|internships)\M'
          OR regexp_replace(lower(COALESCE(job_employment, '')), '[^a-z]', '', 'g')
             IN ('intern', 'interns', 'internship', 'internships') THEN 'intern'
        -- Do not mistake senior/staff/lead roles for junior or mid-level roles.
        WHEN job_title ~* '\m(senior|sr|staff|principal|lead|manager|director|architect)\M' THEN 'other'
        WHEN job_title ~* '\m(software( development)? engineer|software developer|sde)\M' THEN
            CASE
                WHEN job_title ~* '\m(software( development)? engineer|software developer|sde)[[:space:]-]+(ii|2)\M'
                  OR job_title ~* '\m(level[[:space:]-]*(2|ii)|sde[[:space:]-]*2|mid[[:space:]-]*level)\M'
                    THEN 'sde-2'
                WHEN job_title ~* '\m(software( development)? engineer|software developer|sde)[[:space:]-]+(i|1)\M'
                  OR job_title ~* '\m(level[[:space:]-]*(1|i)|sde[[:space:]-]*1|junior|jr|entry[[:space:]-]*level|new[[:space:]-]*grad(uate)?)\M'
                    THEN 'sde-1'
                ELSE 'other'
            END
        -- Common compact title spellings (SDE1, SDE-2).
        WHEN job_title ~* '\msde[[:space:]-]*2\M' THEN 'sde-2'
        WHEN job_title ~* '\msde[[:space:]-]*1\M' THEN 'sde-1'
        ELSE 'other'
    END
$$;

CREATE FUNCTION job_country_codes(job_location TEXT)
RETURNS TEXT[] LANGUAGE SQL IMMUTABLE PARALLEL SAFE AS $$
    SELECT array_remove(ARRAY[
        CASE WHEN replace(COALESCE(job_location, ''), '.', '') ~* '\m(united states( of america)?|usa|us|san francisco|new york|seattle|boston|chicago|los angeles|san diego|denver|atlanta|dallas|houston|washington dc)\M'
            THEN 'US' END,
        CASE WHEN COALESCE(job_location, '') ~* '\m(india|bharat|bengaluru|bangalore|mumbai|delhi|hyderabad|pune|chennai|kolkata|noida|gurugram|gurgaon|ahmedabad|jaipur)\M'
            THEN 'IN' END
    ], NULL)
$$;

ALTER TABLE job_postings
    ADD COLUMN role_category TEXT GENERATED ALWAYS AS (job_role_category(title, employment_type)) STORED,
    ADD COLUMN country_codes TEXT[] GENERATED ALWAYS AS (job_country_codes(location)) STORED;

CREATE INDEX idx_job_postings_active_role_category ON job_postings(role_category) WHERE is_active = TRUE;
CREATE INDEX idx_job_postings_active_countries ON job_postings USING GIN(country_codes) WHERE is_active = TRUE;

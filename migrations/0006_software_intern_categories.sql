-- The role dropdown partitions jobs into Software intern / SDE-1 / SDE-2 / Others.
CREATE OR REPLACE FUNCTION job_role_category(job_title TEXT, job_employment TEXT)
RETURNS TEXT LANGUAGE SQL IMMUTABLE PARALLEL SAFE AS $$
    SELECT CASE
        WHEN job_title ~* '\m(intern|interns|internship|internships)\M'
          OR regexp_replace(lower(COALESCE(job_employment, '')), '[^a-z]', '', 'g')
             IN ('intern', 'interns', 'internship', 'internships') THEN
            CASE WHEN job_is_software_engineering(job_title) THEN 'intern' ELSE 'other' END
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
        WHEN job_title ~* '\msde[[:space:]-]*2\M' THEN 'sde-2'
        WHEN job_title ~* '\msde[[:space:]-]*1\M' THEN 'sde-1'
        ELSE 'other'
    END
$$;

-- Replacing an immutable function does not recalculate existing generated values.
-- Only non-software internships change categories; explicitly refresh those rows.
UPDATE job_postings SET employment_type = employment_type
    WHERE role_category = 'intern' AND is_software_engineering = FALSE;

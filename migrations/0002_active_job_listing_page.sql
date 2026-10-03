CREATE INDEX idx_job_postings_active_page
    ON job_postings (company, title, id)
    WHERE is_active = TRUE;

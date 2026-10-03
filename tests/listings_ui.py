"""Browser regressions. Run: python3 tests/listings_ui.py (requires Playwright + Chromium)."""
import json
from pathlib import Path
import unittest
from urllib.parse import parse_qs, urlparse

from playwright.sync_api import sync_playwright

HTML = (Path(__file__).parents[1] / "templates/jobs.html").read_text()


def job(number):
    return {"id": number, "title": f"Engineer {number}", "company": "Example",
            "location": "Bengaluru", "url": "https://example.test/job", "workplace_type": None,
            "role_category": ["intern", "sde-1", "sde-2", "other"][number % 4],
            "country_codes": ["IN"], "is_software_engineering": number % 5 == 4}


class ListingsUI(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.playwright = sync_playwright().start()
        cls.browser = cls.playwright.chromium.launch(headless=True)

    @classmethod
    def tearDownClass(cls):
        cls.browser.close()
        cls.playwright.stop()

    def setUp(self):
        self.page = self.browser.new_page()
        self.total = 60
        self.requests = []

        def serve(route):
            url = urlparse(route.request.url)
            if url.path == "/api/job-postings":
                params = parse_qs(url.query)
                self.requests.append(params)
                page = int(params["page"][0])
                size = int(params["page_size"][0])
                start = (page - 1) * size
                jobs = [job(i) for i in range(start, min(start + size, self.total))]
                route.fulfill(content_type="application/json", body=json.dumps({
                    "jobs": jobs, "total": self.total, "page": page, "page_size": size}))
            else:
                route.fulfill(content_type="text/html", body=HTML)

        self.page.route("http://job-notif.test/**", serve)
        self.page.goto("http://job-notif.test/")
        self.page.wait_for_function("document.querySelector('#result-count').textContent === '60 open roles'")

    def tearDown(self):
        self.page.close()

    def test_shrinking_results_refetch_valid_page(self):
        self.page.click("#next-page")
        self.page.wait_for_function("document.querySelector('#page-status').textContent.includes('Page 2 of 3')")
        self.page.click("#next-page")
        self.page.wait_for_function("document.querySelector('#page-status').textContent.includes('Page 3 of 3')")
        self.total = 26
        self.page.click("#refresh")
        self.page.wait_for_function("document.querySelector('#page-status').textContent.includes('Page 2 of 2')")
        self.assertEqual(self.page.locator("#listings-body tr").count(), 1)
        self.assertIn("Engineer 25", self.page.locator("#listings-body").inner_text())
        self.assertEqual([r["page"][0] for r in self.requests[-2:]], ["3", "2"])
        self.assertTrue(self.page.locator("#next-page").is_disabled())

    def test_inclusive_filters_are_explicit_and_clear_resets_them(self):
        self.select_filter("#location-filter", "IN")
        self.select_filter("#role-filter", "sde-1")
        self.select_filter("#workplace-filter", "remote")
        with self.page.expect_response("**/api/job-postings?*"):
            self.page.check("#include-global")
        with self.page.expect_response("**/api/job-postings?*"):
            self.page.check("#include-unknown-workplace")
        self.page.wait_for_function("document.querySelector('#result-count').textContent === '60 open roles'")
        self.assertEqual(self.requests[-1]["include_global"], ["true"])
        self.assertEqual(self.requests[-1]["include_unknown_workplace"], ["true"])
        self.assertIn("eligibility unverified", self.page.locator(".filter-options").inner_text())
        self.assertIn("Not specified", self.page.locator("#listings-body").inner_text())
        with self.page.expect_response("**/api/job-postings?*"):
            self.page.click("button[type=reset]")
        self.page.wait_for_function("document.querySelector('#location-filter').value === ''")
        self.page.wait_for_function("document.querySelector('#result-count').textContent === '60 open roles'")
        self.assertFalse(self.page.locator("#include-global").is_checked())
        self.assertFalse(self.page.locator("#include-unknown-workplace").is_checked())
        self.assertNotIn("include_global", self.requests[-1])
        self.assertNotIn("include_unknown_workplace", self.requests[-1])
        self.assertEqual(self.requests[-1], {"page": ["1"], "page_size": ["25"]})
        self.assertEqual(self.page.locator("#role-filter").input_value(), "")
        self.assertEqual(self.page.locator("#workplace-filter").input_value(), "")

    def select_filter(self, selector, value):
        with self.page.expect_response("**/api/job-postings?*"):
            self.page.select_option(selector, value)
        self.page.wait_for_function("document.querySelector('#result-count').textContent === '60 open roles'")

    def test_role_and_country_options_send_exact_params(self):
        self.assertEqual(self.page.locator("#role-filter option").all_text_contents(),
                         ["All roles", "Software intern", "SDE-1", "SDE-2", "Others"])
        self.assertEqual(self.page.locator("#location-filter option").all_text_contents(),
                         ["All countries", "USA", "India"])
        for role in ["intern", "sde-1", "sde-2", "other", ""]:
            self.select_filter("#role-filter", role)
            for country in ["US", "IN", ""]:
                self.select_filter("#location-filter", country)
                expected = {"page": ["1"], "page_size": ["25"]}
                if role:
                    expected["role_category"] = [role]
                if country:
                    expected["country"] = [country]
                self.assertEqual(self.requests[-1], expected)

    def test_dropdown_changes_load_immediately_and_reset_page(self):
        self.page.evaluate("""() => {
            const fetch = window.fetch;
            window.filterFetches = [];
            window.fetch = (...args) => {
                window.filterFetches.push(args[0]);
                return fetch(...args);
            };
        }""")
        for selector, value, param in [("#role-filter", "sde-2", "role_category"),
                                       ("#location-filter", "US", "country")]:
            self.page.click("#next-page")
            self.page.wait_for_function("document.querySelector('#page-status').textContent.includes('Page 2 of 3')")
            with self.page.expect_response("**/api/job-postings?*"):
                urls = self.page.evaluate("""({selector, value}) => {
                    window.filterFetches = [];
                    const select = document.querySelector(selector);
                    select.value = value;
                    select.dispatchEvent(new Event('change', {bubbles: true}));
                    return window.filterFetches.slice();
                }""", {"selector": selector, "value": value})
            self.assertEqual(len(urls), 1, "Change must fetch synchronously, without debounce")
            params = parse_qs(urlparse(urls[0]).query)
            self.assertEqual(params["page"], ["1"])
            self.assertEqual(params[param], [value])
            self.page.wait_for_function("document.querySelector('#page-status').textContent.includes('Page 1 of 3')")

    def test_category_labels_preserve_title_location_and_metadata(self):
        for index, label in enumerate(["Software intern", "SDE-1", "SDE-2", "Others"]):
            row = self.page.locator("#listings-body tr").nth(index)
            self.assertEqual(row.locator(".role-meta").inner_text(), label)
            self.assertEqual(row.locator(".role a").inner_text(), f"Engineer {index}")
            self.assertEqual(row.locator(".location").inner_text(), "Bengaluru")
        note = self.page.locator(".filter-note").inner_text()
        self.assertIn("explicit title levels", note)
        self.assertIn("junior/mid-level", note)
        self.assertIn("unknown software levels", note)
        self.assertIn("software-related internships only", note)
        self.assertIn("all remaining jobs appear under Others", note)
        self.assertIn("not an eligibility guarantee", note)

    def test_software_job_without_known_level_has_others_label(self):
        row = self.page.locator("#listings-body tr").nth(19)
        self.assertEqual(row.locator(".role-meta").inner_text(), "Others")

    def test_shrink_to_zero_shows_empty_result_not_stale_page(self):
        self.page.click("#next-page")
        self.page.wait_for_function("document.querySelector('#page-status').textContent.includes('Page 2 of 3')")
        self.total = 0
        self.page.click("#refresh")
        self.page.wait_for_function("document.querySelector('#page-status').textContent.includes('Page 1 of 1')")
        self.assertIn("No open roles match", self.page.locator("#listings-body").inner_text())
        self.assertTrue(self.page.locator("#previous-page").is_disabled())
        self.assertTrue(self.page.locator("#next-page").is_disabled())


if __name__ == "__main__":
    unittest.main()

"""Browser regressions. Run: python3 tests/listings_ui.py (requires Playwright + Chromium)."""
import json
from pathlib import Path
import unittest
from urllib.parse import parse_qs, urlparse

from playwright.sync_api import sync_playwright

HTML = (Path(__file__).parents[1] / "templates/jobs.html").read_text()


def job(number):
    return {"id": number, "title": f"Engineer {number}", "company": "Example",
            "location": "Bengaluru", "url": "https://example.test/job", "workplace_type": None}


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
        self.page.fill("#location-filter", "India")
        self.page.check("#include-global")
        self.page.check("#include-unknown-workplace")
        self.page.wait_for_function("document.querySelector('#result-count').textContent === '60 open roles'")
        self.page.wait_for_timeout(350)
        self.assertEqual(self.requests[-1]["include_global"], ["true"])
        self.assertEqual(self.requests[-1]["include_unknown_workplace"], ["true"])
        self.assertIn("eligibility unverified", self.page.locator(".filter-options").inner_text())
        self.assertIn("Not specified", self.page.locator("#listings-body").inner_text())
        self.page.click("button[type=reset]")
        self.page.wait_for_function("document.querySelector('#location-filter').value === ''")
        self.page.wait_for_function("document.querySelector('#result-count').textContent === '60 open roles'")
        self.assertFalse(self.page.locator("#include-global").is_checked())
        self.assertFalse(self.page.locator("#include-unknown-workplace").is_checked())
        self.assertNotIn("include_global", self.requests[-1])
        self.assertNotIn("include_unknown_workplace", self.requests[-1])

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

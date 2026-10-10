import tempfile
import unittest
from pathlib import Path

from tools.p10.compare import G4_BASELINE_DATE, Trace, initialize_rust_repository


class G4ComparisonTests(unittest.TestCase):
    def test_rust_baseline_commit_is_reproducible_and_its_date_is_traced(self):
        with tempfile.TemporaryDirectory(prefix="il-g4-baseline-") as temporary:
            root = Path(temporary)
            commits = []
            traces = []
            for identity in ("first", "second"):
                repository = root / identity
                repository.mkdir()
                (repository / "main.rs").write_text("fn main() {}\n", encoding="utf-8")
                trace = Trace(root / f"{identity}-trace")
                _, commit = initialize_rust_repository(trace, repository)
                commits.append(commit)
                traces.append(trace)

            self.assertEqual(len(commits[0]), 40)
            self.assertEqual(commits[0], commits[1])
            for trace in traces:
                commit_event = next(
                    event for event in trace.events
                    if event["request"]["argv"][-2:] == ["-m", "test: pin G4 baseline"]
                )
                self.assertEqual(commit_event["request"]["environment"], {
                    "GIT_AUTHOR_DATE": G4_BASELINE_DATE,
                    "GIT_COMMITTER_DATE": G4_BASELINE_DATE,
                })


if __name__ == "__main__":
    unittest.main()

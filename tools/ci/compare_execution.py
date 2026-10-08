"""Compare actual debug/release interpreter execution identities."""
import json
from pathlib import Path
import sys


def main():
    if len(sys.argv) != 1:
        raise SystemExit("profile comparison accepts no arguments")
    reports = [json.loads(Path(path).read_text(encoding="utf-8")) for path in
               ["build/execution_cli_debug_report.json", "build/execution_cli_report.json"]]
    if not all(report["passed"] and report["cases"] for report in reports):
        raise SystemExit("E_EVIDENCE_INCOMPLETE: both actual execution suites must pass")
    if reports[0]["contract_sha256"] != reports[1]["contract_sha256"]:
        raise SystemExit("E_EVIDENCE_INCOMPLETE: profile suites used different contracts")
    if reports[0]["cases"] != reports[1]["cases"]:
        raise SystemExit("E_STATE_INCONSISTENT: debug and release execution differ")
    print(json.dumps({"passed": True, "count": len(reports[0]["cases"]),
                      "contract_sha256": reports[0]["contract_sha256"]}))


if __name__ == "__main__":
    main()

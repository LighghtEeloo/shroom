#!/usr/bin/env python3
"""Compile upstream routing and query parsing to check storage-scope compatibility.

Usage: python3 check-router.py /path/to/cua
No Lume service, VM, network connection, or third-party Python package is needed.
"""

from pathlib import Path
import subprocess
import sys
import tempfile


def main() -> None:
    upstream = Path(sys.argv[1])
    relative = Path("libs/lume/src/Server/Server.swift")
    source = (upstream / relative).read_text()
    start = source.index("    private struct Route {")
    end = source.index("    // MARK: - Properties", start)
    route = source[start:end].replace("private struct Route", "struct Route", 1)
    parser_start = source.index("    private func extractQueryParam(")
    parser_end = source.index("    private func extractPathParams(", parser_start)
    parser = source[parser_start:parser_end].replace("private func", "func", 1)
    patch = Path(__file__).resolve().parents[3] / "integrations/lume/http-routing.patch"
    subprocess.run(["git", "apply", "--check", str(patch)], cwd=upstream, check=True)

    with tempfile.TemporaryDirectory(prefix="shroom-lume-router-") as directory:
        temporary = Path(directory)
        copied = temporary / relative
        copied.parent.mkdir(parents=True)
        copied.write_text(source)
        # Apply the checked-in patch to a temporary copy, never the upstream checkout.
        subprocess.run(["git", "apply", str(patch)], cwd=temporary, check=True)
        patched_source = copied.read_text()
        patched_start = patched_source.index("    private struct Route {")
        patched_end = patched_source.index("    // MARK: - Properties", patched_start)
        patched_route = patched_source[patched_start:patched_end].replace(
            "private struct Route", "struct PatchedRoute", 1
        )
        program = """
import Foundation
struct HTTPRequest { let method: String; let path: String }
struct HTTPResponse {}
""" + route + patched_route + "struct QueryParser {\n" + parser + "}\n" + """
let original = Route(method: "GET", path: "/lume/vms", handler: { _ in HTTPResponse() })
let query = HTTPRequest(method: "GET", path: "/lume/vms?storage=%2Ftmp%2Fshroom")
precondition(!original.matches(query), "Expected the reviewed upstream routing defect")
print("upstream: storage-scoped listing routing defect reproduced")
let cases: [(String, String, String, Bool)] = [
    ("/lume/vms", "GET", "/lume/vms", true),
    ("/lume/vms", "GET", "/lume/vms?storage=%2Ftmp%2Fshroom", true),
    ("/lume/vms/:name", "GET", "/lume/vms/project?storage=%2Ftmp%2Fshroom", true),
    ("/lume/vms/:name", "GET", "/lume/vms/project/extra", false),
    ("/lume/vms/:name/run", "GET", "/lume/vms/project/run?storage=%2Ftmp", true),
    ("/lume/vms", "DELETE", "/lume/vms?storage=%2Ftmp", false),
]
for (pattern, method, path, expected) in cases {
    let candidate = PatchedRoute(method: "GET", path: pattern, handler: { _ in HTTPResponse() })
    precondition(candidate.matches(HTTPRequest(method: method, path: path)) == expected)
}
print("patched: all six routing cases passed")
let parser = QueryParser()
let formEncoded = HTTPRequest(method: "GET", path: "/lume/vms?storage=%2Ftmp%2Fspace+here")
precondition(parser.extractQueryParam(request: formEncoded, name: "storage") == "/tmp/space+here")
let encoded = HTTPRequest(
    method: "GET", path: "/lume/vms?storage=%2Ftmp%2Fshroom%20lune%20%26%20friends%2B")
precondition(parser.extractQueryParam(request: encoded, name: "storage") == "/tmp/shroom lune & friends+")
let doubleEncoded = HTTPRequest(method: "GET", path: "/lume/vms?storage=%2Ftmp%2Fliteral%252F")
precondition(parser.extractQueryParam(request: doubleEncoded, name: "storage") == "/tmp/literal/")
print("upstream: percent-encoding required for spaces; literal percent must be rejected")
"""
        swift = temporary / "main.swift"
        executable = temporary / "check-router"
        swift.write_text(program)
        subprocess.run(
            ["swiftc", "-module-cache-path", str(temporary / "cache"), str(swift), "-o", str(executable)],
            check=True,
        )
        subprocess.run([str(executable)], check=True)


if __name__ == "__main__":
    main()

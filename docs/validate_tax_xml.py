#!/usr/bin/env python3
"""Validate an exported EPO XML against the pinned official XSD, without network access."""
import argparse
import pathlib
import shutil
import subprocess
import sys
import xml.etree.ElementTree as ET


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("document", type=pathlib.Path)
    args = parser.parse_args()
    path = args.document.resolve()
    if not path.is_file() or path.stat().st_size > 10 * 1024 * 1024:
        parser.error("Provide an existing XML file no larger than 10 MiB.")
    data = path.read_bytes()
    try:
        text = data.decode("utf-8-sig")
    except UnicodeDecodeError:
        parser.error("This MVP validation helper accepts UTF-8 XML only.")
    if "<!DOCTYPE" in text.upper() or "<!ENTITY" in text.upper():
        parser.error("DTD and entity declarations are not accepted.")
    try:
        root = ET.fromstring(text)
    except ET.ParseError as error:
        parser.error(str(error))
    supported = {"DPHDP3", "DPHKH1", "DPHSHV"}
    if root.tag != "Pisemnost" or len(root) != 1 or root[0].tag not in supported:
        parser.error("Expected Pisemnost containing exactly one supported EPO form.")
    validator = shutil.which("xmllint")
    if not validator:
        parser.error("Install libxml2/xmllint; macOS includes /usr/bin/xmllint.")
    form = root[0].tag
    schema = pathlib.Path(__file__).parent / "schemas" / (form.lower() + ".xsd")
    result = subprocess.run([validator, "--nonet", "--noout", "--schema", str(schema), str(path)])
    if result.returncode:
        return result.returncode
    print(f"{form}: schema-valid. Tax correctness has not been assessed; nothing was submitted.")
    return 0


if __name__ == "__main__":
    sys.exit(main())

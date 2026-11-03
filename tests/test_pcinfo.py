import pytest

from update_check import pcinfo
from update_check.pcinfo import as_text, bios_version, model, size, windows_name


def test_windows_11_is_told_apart_by_its_build_number():
    # The registry still says "Windows 10" on Windows 11.
    assert windows_name("Windows 10 Pro", 26300) == "Windows 11 Pro"
    assert windows_name("Windows 10 Home", 19045) == "Windows 10 Home"
    assert windows_name("Windows Server 2025 Datacenter", 26100) == "Windows Server 2025 Datacenter"


@pytest.mark.parametrize(
    ("bios", "expected"),
    [
        ({"SystemManufacturer": "Dell Inc.", "SystemProductName": "XPS 15 9530"}, "Dell Inc. XPS 15 9530"),
        (
            {"SystemManufacturer": "HP", "SystemProductName": "HP EliteBook 840 G8 Notebook PC"},
            "HP EliteBook 840 G8 Notebook PC",
        ),
        (
            {
                "SystemManufacturer": "LENOVO", "SystemProductName": "21HMCTO1WW",
                "SystemVersion": "ThinkPad X1 Carbon Gen 11",
            },
            "LENOVO ThinkPad X1 Carbon Gen 11",
        ),
        (
            {
                "SystemManufacturer": "System manufacturer", "SystemProductName": "System Product Name",
                "BaseBoardManufacturer": "ASUSTeK COMPUTER INC.", "BaseBoardProduct": "ROG STRIX B550-F GAMING",
            },
            "ASUSTeK COMPUTER INC. ROG STRIX B550-F GAMING",
        ),
        ({}, ""),
    ],
)
def test_model_names_maker_and_model_or_the_mainboard_of_a_pc_built_from_parts(bios, expected):
    assert model(bios) == expected


def test_bios_version_comes_with_its_release_date():
    assert bios_version({"BIOSVersion": "22.02", "BIOSReleaseDate": "09/25/2025"}) == "22.02 · Sep 25, 2025"
    assert bios_version({"BIOSVersion": "1.12.0", "BIOSReleaseDate": "sometime"}) == "1.12.0 · sometime"
    assert bios_version({"BIOSVersion": "F12"}) == "F12"


@pytest.mark.parametrize(
    ("count", "text"),
    [
        (32 * 1024**3, "32 GB"),
        (int(15.9 * 1024**3), "15.9 GB"),
        (8 * 1024**3, "8 GB"),
        (954 * 1024**3, "954 GB"),
        (1862 * 1024**3, "1.82 TB"),
    ],
)
def test_sizes_are_given_to_three_figures(count, text):
    assert size(count) == text


def test_the_clipboard_gets_a_line_per_row():
    sections = [("Device", [("Name", "PC"), ("Processor", "Intel\n12 cores")]), ("Firmware", [("TPM", "2.0")])]
    assert as_text(sections) == "Device\nName: PC\nProcessor: Intel, 12 cores\n\nFirmware\nTPM: 2.0"


def test_reading_this_pc_leaves_no_row_empty():
    sections = pcinfo.gather()
    assert sections[0][0] == "Device"
    assert all(rows and all(value for _, value in rows) for _, rows in sections)

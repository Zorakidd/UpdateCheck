from datetime import date

import pytest

from update_check import pcinfo
from update_check.pcinfo import (
    Section,
    age,
    as_text,
    bios_page,
    bios_version,
    first_installed,
    management,
    model,
    windows_name,
)

ASROCK = {"SystemManufacturer": "ASRock", "SystemProductName": "Z690 PG Velocita"}


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


def test_bios_version_comes_with_its_release_date_and_age():
    today = date(2026, 11, 3)
    assert bios_version({"BIOSVersion": "22.02", "BIOSReleaseDate": "09/25/2025"}, today) == (
        "22.02 · Sep 25, 2025 (13 months old)"
    )
    assert bios_version({"BIOSVersion": "1.12.0", "BIOSReleaseDate": "sometime"}, today) == "1.12.0 · sometime"
    assert bios_version({"BIOSVersion": "F12"}, today) == "F12"


@pytest.mark.parametrize(
    ("day", "text"),
    [
        (date(2026, 10, 20), "less than a month old"),
        (date(2026, 10, 3), "1 month old"),
        (date(2025, 9, 25), "13 months old"),
        (date(2023, 1, 1), "3 years old"),
    ],
)
def test_age_is_given_in_months_then_years(day, text):
    assert age(day, date(2026, 11, 3)) == text


def test_brand_pcs_get_their_makers_support_site_and_others_a_search_for_their_model():
    assert bios_page({"SystemManufacturer": "Dell Inc.", "SystemProductName": "XPS 15"}).startswith(
        "https://www.dell.com/",
    )
    assert bios_page({"SystemManufacturer": "HP", "SystemProductName": "HP EliteBook"}).startswith(
        "https://support.hp.com/",
    )
    assert bios_page({"SystemManufacturer": "Microsoft Corporation"}) == "ms-settings:windowsupdate"
    assert bios_page(ASROCK) == "https://www.bing.com/search?q=ASRock+Z690+PG+Velocita+BIOS+update"


@pytest.mark.parametrize(
    ("domain", "tenant", "text"),
    [
        (None, None, "Not managed by an organization"),
        ("CONTOSO", None, "CONTOSO (Active Directory)"),
        (None, "Contoso", "Contoso (Microsoft Entra ID)"),
        (None, "", "Microsoft Entra ID"),
        ("CONTOSO", "Contoso", "Contoso (Microsoft Entra ID)\nCONTOSO (Active Directory)"),
    ],
)
def test_who_manages_the_pc(domain, tenant, text):
    assert management(domain, tenant) == text


def test_installed_on_is_the_first_installation_not_the_last_upgrade():
    # Windows 10 installed in 2021, upgraded to Windows 11 in 2023 and to a new version in 2026.
    first, upgrade, current = 1_620_000_000, 1_690_000_000, 1_771_623_207
    assert first_installed(current, [upgrade, first]) == first
    assert first_installed(current, []) == current
    assert first_installed(current, [None, 0, "?"]) == current
    assert first_installed(None, []) is None


def test_the_clipboard_gets_a_line_per_row():
    sections = [
        Section("Device", [("Name", "PC"), ("Processor", "Intel\n12 cores")]),
        Section("Firmware", [("TPM", "2.0")], action=("Find BIOS updates", "https://example.com")),
    ]
    assert as_text(sections) == "Device\nName: PC\nProcessor: Intel, 12 cores\n\nFirmware\nTPM: 2.0"


def test_reading_this_pc_leaves_no_row_empty():
    sections = pcinfo.gather(updates="Up to date")
    assert [section.title for section in sections][:1] == ["Device"]
    assert all(section.rows and all(value for _, value in section.rows) for section in sections)
    windows = next(section for section in sections if section.title == "Windows")
    assert ("Updates", "Up to date") in windows.rows
    assert any(label == "Managed by" for label, _ in windows.rows)


def test_restart_pending_is_read_without_error():
    assert isinstance(pcinfo.restart_pending(), bool)

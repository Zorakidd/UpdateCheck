import json
import time

import pytest

from update_check import graphics
from update_check.graphics import (
    AMD,
    CHECK_FAILED,
    CHECKING,
    INTEL,
    NO_DRIVER,
    NOT_CHECKED,
    NVIDIA,
    UP_TO_DATE,
    UPDATE_AVAILABLE,
    Card,
    DriverChecks,
    Release,
    nvidia_product_id,
    parse_nvidia_driver,
    vendor_of,
)

RTX_4070_LAPTOP = Card(NVIDIA, "NVIDIA GeForce RTX 4070 Laptop GPU", "32.0.16.1692")
IRIS_XE = Card(INTEL, "Intel(R) Iris(R) Xe Graphics", "32.0.101.7084")
LATEST = Release("616.92", "https://www.nvidia.com/en-us/drivers/details/278454/")

PRODUCTS = b"""<?xml version="1.0" encoding="UTF-8"?><LookupValueSearch><LookupValues>
<LookupValue ParentID="101"><Name>GeForce GTX 1060</Name><Value>817</Value></LookupValue>
<LookupValue ParentID="102"><Name>GeForce GTX 1060</Name><Value>821</Value></LookupValue>
<LookupValue ParentID="127"><Name>NVIDIA GeForce RTX 4070</Name><Value>1015</Value></LookupValue>
<LookupValue ParentID="129"><Name>GeForce RTX 4070 Laptop GPU</Name><Value>1006</Value></LookupValue>
</LookupValues></LookupValueSearch>"""


def wait_for(checks: DriverChecks) -> None:
    deadline = time.monotonic() + 5
    while checks.busy and time.monotonic() < deadline:
        checks.collect()
        time.sleep(0.01)


@pytest.mark.parametrize(
    ("driver_version", "version"),
    [("32.0.16.1692", "616.92"), ("31.0.15.3623", "536.23"), ("30.0.14.7141", "471.41")],
)
def test_nvidia_version_comes_from_the_windows_driver_version(driver_version, version):
    assert Card(NVIDIA, "GPU", driver_version).version == version


def test_other_vendors_keep_the_windows_driver_version():
    assert IRIS_XE.version == "32.0.101.7084"


def test_vendor_comes_from_the_pci_vendor_id():
    assert vendor_of(r"PCI\VEN_10DE&DEV_2820&SUBSYS_C1CC144D&REV_A1") == NVIDIA
    assert vendor_of(r"PCI\VEN_1002&DEV_73BF") == AMD
    assert vendor_of(r"pci\ven_8086&dev_a7a0") == INTEL
    assert vendor_of(r"ROOT\BasicDisplay") is None


def test_detection_returns_cards():
    assert all(isinstance(card, Card) for card in graphics.detect())


@pytest.mark.parametrize(
    ("name", "product"),
    [
        ("NVIDIA GeForce RTX 4070 Laptop GPU", "1006"),
        ("NVIDIA GeForce RTX 4070", "1015"),
        ("NVIDIA GeForce GTX 1060 6GB", "817"),
        ("NVIDIA GeForce RTX 9999", None),
    ],
)
def test_cards_are_found_in_nvidias_product_list(name, product):
    assert nvidia_product_id(PRODUCTS, name) == product


def test_latest_nvidia_driver_is_read_from_the_lookup():
    encoded_url = "https%3A%2F%2Fwww.nvidia.com%2Fen-us%2Fdrivers%2Fdetails%2F278454%2F"
    response = {"Success": "1", "IDS": [{"downloadInfo": {"Version": "616.92", "DetailsURL": encoded_url}}]}

    assert parse_nvidia_driver(json.dumps(response).encode()) == LATEST
    assert parse_nvidia_driver(b'{"Success": "0", "IDS": []}') is None
    with pytest.raises(ValueError, match="Unexpected"):
        parse_nvidia_driver(json.dumps({"IDS": [{"downloadInfo": {"Version": "<html>"}}]}).encode())


def test_nvidia_card_is_checked_online_and_others_are_not():
    checks = DriverChecks(detect=lambda: [RTX_4070_LAPTOP, IRIS_XE], look_up=lambda card: LATEST)

    assert checks.refresh()
    assert checks.status(RTX_4070_LAPTOP) == (CHECKING, None)
    assert checks.status(IRIS_XE) == (NOT_CHECKED, None)
    wait_for(checks)
    assert checks.status(RTX_4070_LAPTOP) == (UP_TO_DATE, LATEST)


def test_update_is_available_until_the_newer_driver_is_installed():
    older = Card(NVIDIA, RTX_4070_LAPTOP.name, "32.0.16.1664")
    installed = [older]
    checks = DriverChecks(detect=lambda: list(installed), look_up=lambda card: LATEST)
    checks.refresh()
    wait_for(checks)
    assert checks.status(older) == (UPDATE_AVAILABLE, LATEST)

    installed[:] = [RTX_4070_LAPTOP]

    assert checks.refresh()
    assert not checks.busy
    assert checks.status(RTX_4070_LAPTOP) == (UP_TO_DATE, LATEST)


def test_failed_lookup_is_retried_after_a_while(monkeypatch):
    attempts = []

    def look_up(card):
        attempts.append(card)
        raise OSError("offline")

    checks = DriverChecks(detect=lambda: [RTX_4070_LAPTOP], look_up=look_up)
    checks.refresh()
    wait_for(checks)
    assert checks.status(RTX_4070_LAPTOP) == (CHECK_FAILED, None)

    checks.refresh()
    assert len(attempts) == 1

    monkeypatch.setattr(DriverChecks, "RETRY_AFTER", 0)
    checks.refresh()
    wait_for(checks)
    assert len(attempts) == 2


def test_card_without_its_driver_needs_one():
    card = Card(AMD, "AMD graphics", "")
    checks = DriverChecks(detect=lambda: [card], look_up=lambda card: LATEST)
    checks.refresh()

    assert not checks.busy
    assert checks.status(card) == (NO_DRIVER, None)

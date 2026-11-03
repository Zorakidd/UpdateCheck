# Windows 11 Upgrade- und Repair-Tool: Anforderungsliste

Nov 3, 2026

## Ziel, Rahmen und Modi

Das Tool soll sich wie TÜV und Werkstatt zugleich verhalten: erst prüfen, dann nur auf Nachfrage reparieren, und das Upgrade kommt ganz zuletzt. Jede Zeile in diesem Dokument trägt eine Priorität: **M** = Muss für die erste brauchbare Version, **S** = Soll, **K** = Kann.

**Drei Modi**

- [ ] **M** Prüfen: liest nur und ändert nichts (Systemcheck, Grafikkarte und Treiber, App-Updates, Zustand der Systemdateien)
- [ ] **M** Reparieren: führt DISM, SFC und Co. aus, jeden Schritt nur nach Bestätigung
- [ ] **S** Upgrade: startet erst, wenn alle harten Blocker grün sind und ein Backup bestätigt ist
- [ ] **M** Der Weekly Reminder startet nur eine Schnellprüfung ohne die langen Scans, nie Reparieren oder Upgrade

**Bewusste Nicht-Ziele**

- Keine Umgehung der Windows-11-Hardwareprüfung (TPM, CPU, Secure Boot), weil Microsoft solche PCs nicht unterstützt und spätere Updates dort nicht garantiert sind
- Keine Treiber von Drittseiten, nur über Windows Update oder die Seite des Herstellers
- Das Tool selbst sendet keine Telemetrie (winget und Windows Update sprechen natürlich weiter mit Microsoft)
- Keine Änderungen an PCs, die eine Firma oder Behörde verwaltet (Gruppenrichtlinien, Intune): dort nur prüfen

**Noch offen**

- [ ] Nur eigene PCs, oder auch fremde (dann braucht es Einwilligung und einen Kundenbericht)?
- [ ] Oberfläche: Konsole, kleines Fenster (WPF oder WinUI) oder beides?
- [ ] Sprache der Oberfläche: Deutsch, Englisch oder umschaltbar?
- [ ] Verteilung: einzelne Datei, Installer oder per winget?

## Prioritäten und Realitätscheck

Fang klein an: Stufe 1 ist eine reine Prüfung mit Ampelbericht, und erst danach kommen Reparatur, Reminder und Upgrade. Alles mit M gehört zur ersten brauchbaren Version (Stufe 1 bis 5), das Upgrade mit S folgt in Stufe 6.

**Realitätscheck**

Fast jede Einzelfunktion gibt es schon: die PC-Integritätsprüfung von Microsoft, winget, DISM, SFC und Windows Update. Dein Mehrwert ist das Zusammenspiel aus verständlicher Auswertung, ruhigem Wochenrhythmus und sicherem Upgrade-Weg. Reiht das Tool nur Befehle aneinander, lohnt es sich nicht.

Kann der PC Windows 11, bietet Windows Update das Upgrade in der Regel ohnehin selbst an. Wertvoll ist der PC, der es nicht kann, und dort darf das Tool nichts erzwingen. Sein Nutzen liegt also im Erklären und Absichern, nicht im Installieren.

Prüfen und Berichten sind harmlos und liefern den größten Teil des Nutzens. Reparatur und Upgrade sind die Teile, bei denen etwas kaputtgehen kann, deshalb kommen sie spät und nur mit Trockenlauf, Wiederherstellungspunkt und Tests in VMs. Der Reminder ist der Stolperstein mit Rechten und Benachrichtigungen, also früh an einem kleinen Beispiel ausprobieren.

**Stufen**

1. Prüfen: Systemcheck, Grafikkarte und Treiber, Version und Support-Status, Ampel in der Konsole. Nur lesen
2. Bericht: Klartext, HTML und JSON, Protokolle, Vergleich zur Vorwoche
3. Apps: winget-Prüfung mit Auswahlliste, Zähler für Windows-Updates
4. Reminder: geplante Aufgabe, eine Benachrichtigung, Später und Überspringen
5. Reparatur: DISM und SFC mit Bestätigung, Wiederherstellungspunkt, Trockenlauf
6. Upgrade: Voraussetzungen, Backup-Bestätigung, Weg über Windows Update, Nachkontrolle, in VMs getestet

Teste Stufe 1 zuerst auf zwei oder drei echten PCs. Ist die Ausgabe dort wirklich nützlich, lohnt sich der Rest.

## Systemcheck: Windows-11-Bereitschaft

Das Tool muss in unter einer Minute sagen, ob der PC Windows 11 kann und was ihn blockiert, und dabei nichts verändern.

**Hardware-Mindestwerte**

- [ ] **M** TPM 2.0 vorhanden und aktiv (`Get-Tpm`: TpmPresent, TpmReady, Version)
- [ ] **M** UEFI statt altem BIOS und Systemplatte mit GPT statt MBR (`Get-Disk`: PartitionStyle)
- [ ] **M** Secure Boot fähig und eingeschaltet (`Confirm-SecureBootUEFI`)
- [ ] **M** Prozessor mit 64 Bit, mindestens 2 Kernen und 1 GHz, auf Microsofts Liste unterstützter CPUs (die Liste ändert sich, also nicht hartkodieren) und mit den Befehlen SSE4.2 und POPCNT (ab 24H2 Pflicht)
- [ ] **M** Mindestens 4 GB RAM und mindestens 64 GB Systemlaufwerk
- [ ] **M** Grafik mit DirectX 12 und WDDM-2.x-Treiber (Details im Abschnitt Grafikkarte und Treiber)
- [ ] **K** Bildschirm größer als 9 Zoll mit mehr als 720p Auflösung
- [ ] **S** Gegenprobe durch Windows Setup selbst: `setup.exe /compat scanonly` vom Installationsmedium. Rückgabe 0xC1900210 heißt keine Probleme, 0xC1900208 heißt Blocker durch App oder Treiber. Die Befunde stehen in CompatData-Dateien unter `C:\$WINDOWS.~BT\Sources\Panther`

**Zustand von Windows**

- [ ] **M** Version, Build und Edition lesen (Registry `CurrentVersion`: DisplayVersion, CurrentBuild, UBR)
- [ ] **M** Support-Status anzeigen: Windows 10 22H2 hat seit 14.10.2025 keinen Regelsupport, die Verbraucher-ESU läuft bis 12.10.2027, Windows 11 24H2 Home und Pro ist seit 13.10.2026 außer Support. Diese Daten nicht im Code einbauen, sondern aus einer kleinen Datei lesen, die sich online aktualisieren lässt
- [ ] **M** Neustart ausstehend oder Updates offen? Beides vor einem Upgrade zuerst abarbeiten
- [ ] **M** Freier Platz auf C: (Richtwert: 30 GB frei, damit Setup und der Rollback-Ordner Windows.old Platz haben)
- [ ] **S** Laptop nur mit angeschlossenem Netzteil upgraden
- [ ] **S** Windows Update erreichbar (Internet, Proxy, Uhrzeit stimmt)
- [ ] **K** Aktivierungsstatus nur lesen (`slmgr /xpr`)

**Secure-Boot-Zertifikate (gerade aktuell)**

- [ ] **M** Status lesen: Registry-Wert `UEFICA2023Status` unter `HKLM\SYSTEM\CurrentControlSet\Control\SecureBoot\Servicing` (NotStarted, InProgress oder Updated), alles außer Updated gelb markieren
- [ ] **S** Erklären statt erschrecken: Die Zertifikate von 2011 sind zwischen Juni und Oktober 2026 abgelaufen. Der PC bootet weiter, bekommt aber keine neuen Schutzmaßnahmen für die frühe Startphase
- [ ] **S** Bei NotStarted auf ein BIOS-Update des Herstellers hinweisen und den BitLocker-Wiederherstellungsschlüssel vorher sichern lassen
- [ ] **K** Update nur nach ausdrücklichem Ja anstoßen (Registry-Wert `AvailableUpdates` auf 0x5944, danach die Aufgabe Secure-Boot-Update starten)

## Grafikkarte und Treiber

Das Tool muss jede Grafikkarte erkennen, die Treiberversion in die Zahl umrechnen, die der Hersteller nennt, und sagen, ob etwas Neueres bereitsteht.

**Erkennen**

- [ ] **M** Alle Grafikadapter auflisten, nicht nur den ersten (`Get-CimInstance Win32_VideoController`: Name, PNPDeviceID, DriverVersion, DriverDate, Status). Laptops haben oft zwei, einen sparsamen für den Alltag und einen starken für Spiele
- [ ] **M** Hersteller über die PCI-ID im PNPDeviceID bestimmen (`VEN_10DE` NVIDIA, `VEN_1002` AMD, `VEN_8086` Intel), nicht über den Namenstext
- [ ] **M** Meldet Windows nur den Microsoft Basic Display Adapter, fehlt der Herstellertreiber: rot markieren
- [ ] **M** Defekte oder deaktivierte Adapter erkennen (ConfigManagerErrorCode ungleich 0)
- [ ] **M** Grafikspeicher nicht aus `AdapterRAM` lesen, der Wert läuft ab 4 GB über. Besser den Registry-Wert `HardwareInformation.qwMemorySize` oder DXGI nehmen
- [ ] **S** DirectX-Funktionsstufe und WDDM-Version lesen (`dxdiag /x` schreibt beides in eine XML-Datei). Windows 11 braucht DirectX 12 und WDDM 2.0 oder neuer

**Treiberversion richtig lesen**

Windows meldet bei Grafiktreibern eine andere Nummer als der Hersteller. Das Tool übersetzt sie so:

| Hersteller | Windows meldet | Hersteller nennt | Umrechnung |
| --- | --- | --- | --- |
| NVIDIA | 32.0.15.6094 | 560.94 | letzte Ziffer des dritten Blocks plus vierter Block, Punkt vor den letzten zwei Ziffern |
| Intel | 31.0.101.5333 | 101.5333 | dritter und vierter Block |
| AMD | 31.0.x.x | Adrenalin-Version wie 25.x.x | keine feste Regel, Version aus der AMD-Software lesen |

- [ ] **M** Treiberdatum anzeigen und das Alter bewerten (Richtwert: älter als 12 Monate gelb, einstellbar)
- [ ] **M** Quelle des Treibers zeigen: Hersteller oder Windows Update (`Win32_PnPSignedDriver`: DriverProviderName, InfName, IsSigned)

**Aktualität prüfen**

- [ ] **M** Windows Update fragen, ob ein Grafiktreiber angeboten wird (Suche über den Update-Agent mit `IsInstalled=0 and Type='Driver'`)
- [ ] **S** Den neuesten Herstellertreiber nur als Hinweis mit Link zur Hersteller-Downloadseite zeigen. Inoffizielle Abfrage-Adressen der Hersteller sind fragil und ändern sich ohne Vorwarnung
- [ ] **S** Vor jedem Treiberwechsel einen Wiederherstellungspunkt anlegen und den alten Treiber sichern (`pnputil /export-driver`)
- [ ] **K** Installation nur über die Hersteller-Software oder Windows Update anstoßen, nie einen Treiber selbst entpacken und einspielen

**Weitere Treiber und Firmware**

- [ ] **M** Geräte mit Fehler finden, also das Ausrufezeichen im Geräte-Manager (`Get-PnpDevice -PresentOnly`, Status Error)
- [ ] **S** Chipsatz, Netzwerk, WLAN, Bluetooth, Audio und Speicher mit Anbieter und Datum in derselben Tabelle zeigen
- [ ] **S** BIOS-Version und -Datum lesen (`Win32_BIOS`) und auf die Support-Seite des PC-Herstellers verlinken. Vor einem BIOS-Update BitLocker aussetzen und den Schlüssel sichern
- [ ] **K** Vor dem Upgrade einen Snapshot aller Treiber als JSON speichern und nachher vergleichen

## Reparatur: DISM, SFC und Co.

Das Tool repariert in fester Reihenfolge, erst DISM, dann SFC, und jeder Schritt, der etwas ändert, braucht ein ausdrückliches Ja. Der Prüfmodus benutzt nur die Varianten, die nichts verändern.

**Nur lesen (gehört in den Prüfmodus)**

- [ ] **M** `DISM /Online /Cleanup-Image /CheckHealth` (schnell: ist das Image als beschädigt markiert?) und `/ScanHealth` (gründlich, dauert Minuten)
- [ ] **M** `sfc /verifyonly` prüft alle geschützten Systemdateien und repariert nichts
- [ ] **S** `DISM /Online /Cleanup-Image /AnalyzeComponentStore` zeigt, ob der Komponentenspeicher aufgeräumt werden sollte
- [ ] **S** Datenträger prüfen: `chkdsk C: /scan` (Online-Scan ohne Neustart), `Get-PhysicalDisk` (HealthStatus) und `Get-StorageReliabilityCounter` (Verschleiß, Lesefehler)
- [ ] **S** Ereignisprotokoll der letzten 7 Tage nach Hardware-Warnsignalen durchzählen (Quellen disk, Ntfs, WHEA-Logger, Kernel-Power), bevor Windows die Schuld bekommt
- [ ] **S** Windows-Sicherheit lesen: Defender an, Signaturen aktuell, Firewall an (`Get-MpComputerStatus`)
- [ ] **S** Wiederherstellungsumgebung prüfen (`reagentc /info`, Status Enabled)

**Reparieren (nur nach Ja)**

- [ ] **M** `DISM /Online /Cleanup-Image /RestoreHealth`. Scheitert es, anbieten, eine Windows-ISO zu mounten und deren install.wim oder install.esd als Quelle zu nehmen (`/Source` zusammen mit `/LimitAccess`)
- [ ] **M** `sfc /scannow`, danach erneut prüfen. Bleibt etwas übrig: erst DISM, dann SFC noch einmal
- [ ] **M** SFC-Ergebnis nicht aus dem Ausgabetext lesen (lokalisiert, UTF-16), sondern aus den `[SR]`-Zeilen im CBS.log (`C:\Windows\Logs\CBS\CBS.log`)
- [ ] **S** Meldet `chkdsk /scan` Fehler: Reparatur (`/spotfix` oder `/f`) nur mit Hinweis auf Neustart und Backup
- [ ] **S** Windows-Update-Reset: Dienste wuauserv, bits und cryptsvc stoppen, die Ordner SoftwareDistribution und catroot2 umbenennen (nicht löschen), Dienste wieder starten
- [ ] **K** Store und Apps reparieren (`wsreset.exe`, `Reset-AppxPackage`)
- [ ] **K** Netzwerk zurücksetzen (`netsh winsock reset`), mit Hinweis auf Neustart
- [ ] **K** `DISM /Online /Cleanup-Image /StartComponentCleanup` zum Aufräumen

**Vor jedem Schritt**

- [ ] **M** Adminrechte prüfen und klar sagen, was fehlt, statt kryptisch zu scheitern
- [ ] **M** Neustart offen? Dann zuerst neu starten, sonst bricht SFC mit der Meldung ab, dass eine Systemreparatur auf einen Neustart wartet
- [ ] **M** Laptop am Netzteil, genug freier Platz und keine laufende Update-Installation
- [ ] **M** Fortschrittsanzeige und Abbrechen-Knopf. Richtwert: DISM und SFC brauchen je 5 bis 30 Minuten, auf einer Festplatte länger
- [ ] **M** Jeder Schritt endet mit einem Ergebnis: Ok, Repariert, Nicht reparierbar, Fehler oder Übersprungen, dazu Dauer und Rohausgabe fürs Log

## Apps und Updates mit winget

Das Tool soll alle installierten Apps gegen winget prüfen, die Updates in einer Liste zeigen und nur das installieren, was du abhakst.

**Vorbereitung**

- [ ] **M** winget vorhanden und funktionsfähig? (`winget --version`). Fehlt es oder ist es kaputt: auf den App Installer im Microsoft Store hinweisen und `Repair-WinGetPackageManager` anbieten
- [ ] **M** Quellen prüfen: winget und msstore erreichbar (`winget source list`)
- [ ] **M** Die Textausgabe von `winget upgrade` nicht parsen, sie ist eine lokalisierte Tabelle und bricht bei Änderungen. Besser das PowerShell-Modul Microsoft.WinGet.Client nutzen (`Get-WinGetPackage`, `Update-WinGetPackage`)

**Prüfen**

- [ ] **M** Alle Apps mit installierter und verfügbarer Version zeigen, getrennt nach Quelle (winget, msstore, unbekannt)
- [ ] **M** Apps mit unbekannter Version separat führen und nur nach Rückfrage aktualisieren (`--include-unknown`)
- [ ] **M** Gepinnte Apps respektieren (`winget pin`) und eine eigene Ausschlussliste als JSON führen. Grafiktreiber-Pakete stehen standardmäßig darauf
- [ ] **S** Apps ohne Paketquelle (von Hand installiert) als eigene Liste zeigen, nur als Hinweis
- [ ] **K** Sicherheitsrelevante Apps (Browser, Office, Java, Fernwartung) in der Liste nach oben sortieren

**Aktualisieren (nur nach Auswahl)**

- [ ] **M** Nichts wird ohne Häkchen aktualisiert
- [ ] **M** Lizenzbedingungen nie stillschweigend akzeptieren: einmal anzeigen und bestätigen lassen, erst dann `--accept-package-agreements` setzen
- [ ] **M** Läuft die App gerade (zum Beispiel der Browser)? Dann fragen, ob sie geschlossen werden darf, und nie selbst Prozesse beenden
- [ ] **M** Pro App ein Ergebnis (Erfolg, Neustart nötig, Fehler, übersprungen). Ein Fehler stoppt nicht die ganze Liste
- [ ] **S** Vorher die App-Liste sichern (`winget export`), damit sie sich später wiederherstellen lässt (`winget import`)
- [ ] **S** Adminrechte erst anfordern, wenn ein Paket sie wirklich braucht, nicht schon beim Start
- [ ] **S** Pro App ein Zeitlimit (Richtwert 10 Minuten) und ein Abbrechen-Knopf

**Windows Update und Store**

- [ ] **M** Ausstehende Windows-Updates zählen und anzeigen (COM-Schnittstelle `Microsoft.Update.Session`)
- [ ] **S** Store-Apps über die Quelle msstore in derselben Liste führen
- [ ] **K** Microsoft 365 aktualisiert sich selbst, das Tool zeigt nur die Version

**Fehlerfälle**

- [ ] **M** Kein Internet, Proxy oder Quelle nicht erreichbar: klare Meldung statt Absturz
- [ ] **M** Die häufigsten winget-Fehlercodes in Klartext übersetzen und den Rohcode ins Log schreiben

## Weekly Reminder

Der Weekly Reminder ist wie ein Hausmeister, der einmal pro Woche leise seine Runde macht und nur klingelt, wenn ihm etwas auffällt: Windows startet ihn selbst, er prüft im Hintergrund, und du bekommst höchstens eine Benachrichtigung pro Woche.

&#91;embedded content: Weekly Reminder: Ablauf von der Planung bis zur Benachrichtigung\]

Ist nichts auffällig, bleibt es still, sonst kommt eine einzige Benachrichtigung mit drei möglichen Antworten.

**Wie er läuft**

- [ ] **M** Geplante Aufgabe in der Windows-Aufgabenplanung statt eigener Dienst oder Autostart-Eintrag, mit wöchentlichem Trigger, Wochentag und Uhrzeit nach Wahl
- [ ] **M** Verpasste Läufe nachholen (`StartWhenAvailable`): war der PC zur Startzeit aus, läuft die Prüfung kurz nach dem nächsten Anmelden
- [ ] **M** Läuft als dein Benutzer, damit Benachrichtigungen sichtbar sind, und mit erhöhten Rechten (`RunLevel Highest`), weil sich TPM und Secure Boot nur mit Adminrechten lesen lassen. Die Einrichtung braucht dafür einmal eine UAC-Abfrage
- [ ] **M** Programm und Skripte der Aufgabe liegen in einem Ordner, den normale Benutzer nicht beschreiben dürfen (zum Beispiel unter Program Files), sonst wird die Aufgabe zum Einfallstor
- [ ] **M** Macht nur die Schnellprüfung (Systemcheck, Grafikkarte und Treiber, App-Updates, offene Windows-Updates, Secure-Boot-Status): kein Fenster, niedrige Priorität, Zeitlimit 5 Minuten, keine doppelten Läufe
- [ ] **M** Die langen Scans (`sfc /verifyonly`, `DISM /ScanHealth`, `chkdsk /scan`) laufen nie im Hintergrund, nur von Hand oder einmal im Monat nach Rückfrage, weil sie Minuten dauern und die Platte belasten
- [ ] **S** Nur starten, wenn Netz da ist und der Laptop nicht im Akkubetrieb läuft

**Was du siehst**

- [ ] **M** Höchstens eine Windows-Benachrichtigung pro Woche, mit dem Ergebnis in einem Satz, zum Beispiel: 3 App-Updates, Grafiktreiber 14 Monate alt, Systemdateien in Ordnung
- [ ] **M** Drei Knöpfe: Jetzt ansehen (öffnet den Bericht), Später (morgen noch einmal erinnern), Diese Woche überspringen
- [ ] **M** Ist alles grün, kommt keine Benachrichtigung, das Ergebnis geht nur ins Verlaufsprotokoll (einstellbar: kurze Meldung Alles in Ordnung)
- [ ] **S** Nicht stören und Fokusassistent respektieren: die Meldung landet dann still im Info-Center
- [ ] **S** Sind Benachrichtigungen für das Tool abgeschaltet, den Hinweis beim nächsten Start des Tools zeigen

**Was es sich merkt**

- [ ] **M** Ein kleiner Zustand pro Benutzer als JSON (`%LOCALAPPDATA%`): letzter Lauf, nächster Termin, verschoben bis, übersprungene Wochen, letztes Ergebnis
- [ ] **M** Später verschiebt um einen Tag und höchstens zweimal pro Woche, danach herrscht Ruhe bis zur nächsten Woche, damit es nicht nervt
- [ ] **S** Rote Befunde (Datenträgerfehler, Windows ohne Updates) dürfen nach 3 Tagen erneut erinnern, gelbe nur wöchentlich

**Steuerung**

- [ ] **M** Einrichten nur nach ausdrücklichem Ja, nie heimlich beim ersten Start
- [ ] **M** Wochentag, Uhrzeit und Ein/Aus im Tool einstellbar. Aus löscht die geplante Aufgabe wirklich
- [ ] **M** Deinstallation entfernt die Aufgabe und den Zustand ohne Reste
- [ ] **S** Beim Start prüfen, ob die Aufgabe noch existiert und richtig eingestellt ist, und nach Nachfrage reparieren
- [ ] **S** Per Kommandozeile steuerbar (`schedule enable`, `schedule disable`, `schedule status`)

## Upgrade-Ablauf

Das Upgrade ist der letzte Schritt: Das Tool startet es erst, wenn alle harten Blocker grün sind und ein Backup bestätigt ist, und es nutzt nur offizielle Wege von Microsoft. Die Prioritäten in diesem Abschnitt gelten innerhalb des Upgrade-Moduls, das selbst erst in Stufe 6 kommt.

&#91;embedded content: Upgrade-Ablauf: Prüfungen vor dem Start, Rollback danach\]

Zwei Prüfungen stehen vor dem Upgrade, und nach dem Start bleibt zehn Tage lang ein Rollback möglich.

**Voraussetzungen (alles muss grün sein)**

- [ ] **M** Systemcheck ohne harte Blocker. Bei Rot zeigt das Tool nur die Liste der Blocker und bietet keinen Upgrade-Knopf an
- [ ] **M** Backup bestätigt: Dateien gesichert, BitLocker-Wiederherstellungsschlüssel angezeigt und von dir als gespeichert bestätigt, Wiederherstellungspunkt angelegt
- [ ] **M** Alle Updates des alten Windows installiert (Windows 10 auf dem letzten Stand 22H2) und der Neustart erledigt
- [ ] **S** Treiber-Snapshot und App-Liste (`winget export`) werden dabei automatisch mitgesichert

**Durchführung**

- [ ] **M** Standardweg ist das Feature-Update, das Windows Update anbietet, weil Windows dort bekannte Probleme einzelner Geräte selbst abfängt
- [ ] **S** Zweitweg sind offizielle Microsoft-Medien (Installationsassistent oder ISO), nie Drittseiten. Bei einer ISO vorher die SHA-256-Prüfsumme mit der Angabe von Microsoft vergleichen
- [ ] **M** Vor dem Start zeigen, was passiert: mehrere Neustarts, Richtwert 30 bis 90 Minuten, Dateien und Apps bleiben erhalten, PC nicht ausschalten
- [ ] **M** Danach hält sich das Tool raus: keine Eingriffe während des Upgrades und der Neustarts
- [ ] **M** Keine Tricks gegen die Hardwareprüfung, weder per Registry noch per umgebautem Medium

**Wenn etwas schiefgeht**

- [ ] **M** Setup-Fehler in Klartext übersetzen: Ergebnis- und Erweiterungscode lesen, die Logs aus `C:\$WINDOWS.~BT\Sources\Panther` auswerten und das Microsoft-Werkzeug SetupDiag darüberlaufen lassen
- [ ] **M** Rollback erklären: Windows bringt dich in den ersten zehn Tagen zurück (Einstellungen, System, Wiederherstellung). Der Ordner Windows.old bleibt bis dahin liegen, das Tool räumt ihn nie selbst auf
- [ ] **K** Rollback-Frist auf Wunsch verlängern (`DISM /Online /Set-OSUninstallWindow`)
- [ ] **K** Notfallseite mit den Schritten für die Wiederherstellungsumgebung als PDF ablegen, damit sie auch lesbar ist, wenn der PC nicht mehr startet

**Nachkontrolle (automatisch nach dem ersten Start)**

- [ ] **M** Version, Build und Aktivierung prüfen
- [ ] **M** Grafikkarte und Treiber neu lesen und mit dem Snapshot von vorher vergleichen
- [ ] **M** App-Liste mit der gesicherten vergleichen und fehlende Apps zur Neuinstallation anbieten (`winget import`)
- [ ] **M** `sfc /verifyonly` laufen lassen und den Windows-Update-Status prüfen
- [ ] **M** Prüfen, ob die geplante Aufgabe des Weekly Reminders noch existiert
- [ ] **S** Bericht Vorher und Nachher erzeugen

**Auch unter Windows 11**

- [ ] **S** Neue Windows-11-Versionen über Windows Update anbieten. Wer noch auf 24H2 Home oder Pro ist, steht außer Support und bekommt den Hinweis in Rot

## Sicherheitsnetz und Datenschutz

Ein Tool, das mit Adminrechten an Windows arbeitet, muss selbst vertrauenswürdig sein: Es fragt vor jeder Änderung, kann alles erst trocken durchspielen und hinterlässt nichts, was Schaden anrichten kann.

**Vor Änderungen**

- [ ] **M** Jede ändernde Aktion sagt vorher in einem Satz, was passiert, und braucht ein ausdrückliches Ja. Für riskante Schritte gibt es kein Alles bestätigen
- [ ] **M** Trockenlauf (`--dry-run`): zeigt, was das Tool tun würde, ohne es zu tun
- [ ] **M** Vor Reparaturen und Upgrades einen Wiederherstellungspunkt anlegen (ist der Systemschutz aus, anbieten, ihn einzuschalten). Windows erlaubt standardmäßig nur einen Punkt pro 24 Stunden, das Tool meldet ehrlich, wenn keiner entstanden ist
- [ ] **M** BitLocker-Status lesen. Vor BIOS-Updates, TPM-Änderungen und Secure-Boot-Updates den Schutz aussetzen lassen und den Wiederherstellungsschlüssel vorher sichern. Der Schlüssel landet nie in Dateien oder Logs des Tools
- [ ] **M** Das Tool löscht nie Nutzerdaten und räumt nur Windows-eigene Ablagen auf, und das nur nach Ja

**Rechte**

- [ ] **M** Adminrechte erst anfordern, wenn eine Aktion sie braucht, mit klarem Grund in der UAC-Abfrage. Der Prüfmodus läuft so weit wie möglich ohne
- [ ] **M** Fehlende Rechte klar melden (was fehlt, wie es sich beheben lässt), statt mit einem kryptischen Fehler abzubrechen
- [ ] **S** Das Tool kennt eine feste, kleine Liste erlaubter Aktionen und führt nie eine frei zusammengesetzte Befehlszeile aus Benutzereingaben aus

**Vertrauen ins Tool**

- [ ] **M** Das Tool lädt zur Laufzeit keinen Code aus dem Internet und führt ihn aus. Datendateien (zum Beispiel die Support-Daten für Windows-Versionen) nur mit Prüfsumme oder Signatur
- [ ] **S** Programm und Installer mit einem Code-Signing-Zertifikat signieren, sonst warnt SmartScreen

**Datenschutz**

- [ ] **M** Das Tool selbst sendet keine Telemetrie und lädt keine Berichte hoch, alles bleibt lokal
- [ ] **M** Berichte und Logs enthalten keine Passwörter oder Schlüssel, beim Export lassen sich Rechner- und Benutzername schwärzen
- [ ] **S** Läuft das Tool auf fremden PCs: Einwilligung einholen, Berichte nur lokal speichern und nach Absprache löschen

## Bericht, Logging und Bedienung

Der Bericht ist das eigentliche Produkt: eine Seite mit Ampelfarben, Klartext und einem nächsten Schritt pro Zeile.

**Bericht**

- [ ] **M** Jede Prüfung liefert dasselbe Ergebnisformat: Status (Grün, Gelb, Rot, Unbekannt, Übersprungen), Titel, Befund in einem Satz, Empfehlung und Rohdaten
- [ ] **M** Oben eine Gesamtampel mit den drei wichtigsten Punkten, Details lassen sich aufklappen
- [ ] **M** Klartext statt Fehlercodes: Jeder Fehler bekommt einen Satz, was er bedeutet, und einen nächsten Schritt. Der Rohcode steht dahinter für den Support
- [ ] **M** Unbekannt ist ein eigener Zustand, etwa wenn Adminrechte fehlen, und wird nie stillschweigend grün
- [ ] **S** Vergleich zur Vorwoche: was ist neu, was ist behoben, was bleibt offen
- [ ] **S** Export als HTML zum Weitergeben und als JSON für Skripte
- [ ] **K** Verlauf der offenen Punkte über die Wochen als kleine Kurve

**Logging**

- [ ] **M** Jeder Lauf schreibt ein Protokoll mit Zeitstempel, Tool-Version, Windows-Build, ausgeführten Schritten, Dauer und der Rohausgabe der Befehle (pro Benutzer unter `%LOCALAPPDATA%`)
- [ ] **M** Alte Protokolle werden nach einer einstellbaren Frist gelöscht (Richtwert 90 Tage)
- [ ] **S** Exit-Codes für Skripte: 0 alles grün, 1 gelb, 2 rot, 3 Fehler im Tool
- [ ] **S** Ein Knopf Support-Paket erstellen: Bericht und Logs als ZIP, ohne Schlüssel und mit geschwärzten Namen

**Bedienung**

- [ ] **M** Eine Hauptseite mit Gesamtampel und den Knöpfen Prüfen, Reparieren und Upgrade. Upgrade ist grau, solange etwas blockiert
- [ ] **M** Sichtbarer Fortschritt mit dem aktuellen Schritt, Abbrechen ist immer möglich
- [ ] **M** Deutsch und Englisch, alle Texte in Sprachdateien statt im Code, Datum und Zahlen nach der Windows-Region
- [ ] **S** Farbe nie als einziges Signal, immer zusätzlich Symbol oder Text. Tastaturbedienung und hoher Kontrast müssen funktionieren
- [ ] **S** Hell oder Dunkel nach der Windows-Einstellung
- [ ] **K** Einfacher Modus für Laien (Ampel und ein Knopf Alles prüfen) und ein Expertenmodus mit Rohdaten

## Technik, Tests und Verteilung

Der Kern ist eine Sammlung kleiner, einzeln testbarer Prüfmodule mit einheitlichem Ergebnis, und Oberfläche, Kommandozeile und Weekly Reminder rufen alle denselben Kern auf.

**Stack und Aufbau**

- [ ] **M** Stack festlegen. Vorschlag: Prüfmodule zuerst als PowerShell-Skripte (Windows PowerShell 5.1 ist auf jedem Windows 10 und 11 schon da, die meisten Befehle in diesem Dokument gibt es dort ohne Zusatzinstallation), später der Kern in C# mit .NET, wenn Oberfläche und Planung dazukommen
- [ ] **M** Ein Modul pro Thema (Systemcheck, Grafik, Treiber, Reparatur, winget, Windows Update, Secure Boot, Bericht), jedes mit demselben Ergebnisformat und einzeln testbar
- [ ] **M** Ein gemeinsamer Kern: Oberfläche, Kommandozeile und geplante Aufgabe enthalten keine eigene Logik, sie rufen nur den Kern auf
- [ ] **M** Externe Programme (sfc, DISM, winget, chkdsk) laufen über eine einzige Schicht, die Exit-Code, Ausgabe, Kodierung und Zeitlimit einfängt
- [ ] **M** Konfiguration als JSON: Schwellenwerte, Ausschlussliste, Reminder-Zeiten, Sprache
- [ ] **M** Support-Daten (Windows-Versionen, Support-Enden, Hinweistexte) liegen getrennt vom Code, damit eine Terminänderung wie die ESU-Verlängerung 2026 kein neues Release braucht
- [ ] **M** Das Tool muss auf einem angeschlagenen PC laufen: keine Abhängigkeit von Zusatzprogrammen, die erst installiert werden müssten. Fehlt winget, wird nur dieser Teil übersprungen

**Tests**

- [ ] **M** Prüfmodule mit aufgezeichneten Beispielausgaben testen (Get-Tpm, winget, sfc), ohne den echten PC anzufassen (Pester für PowerShell, xUnit für C#)
- [ ] **M** Testmatrix in virtuellen Maschinen: Windows 10 22H2, Windows 11 25H2 und 26H2, Home und Pro, mit und ohne TPM und Secure Boot
- [ ] **M** Reparatur und Upgrade nur in VMs mit Snapshot ausprobieren, nie zuerst am eigenen PC
- [ ] **S** Eine Hyper-V-VM der Generation 1 als Testfall für den alten BIOS-PC (sie hat kein UEFI und kein Secure Boot)
- [ ] **S** In einer VM absichtlich eine Systemdatei beschädigen, um den SFC- und DISM-Weg zu testen
- [ ] **S** Weekly Reminder testen: Aufgabe anlegen, Uhr vorstellen, einen verpassten Lauf simulieren, Benachrichtigung prüfen
- [ ] **K** Unit-Tests ohne Systemzugriff in einer Build-Pipeline laufen lassen

**Verteilung und Updates**

- [ ] **M** Verteilung festlegen. Vorschlag: signierter Installer plus winget-Paket, weil winget die Prüfsumme kontrolliert und das Tool so selbst per winget aktuell bleibt
- [ ] **M** Versionsnummer nach SemVer, sichtbar in Bericht und Log
- [ ] **S** Neue Tool-Version nur als Hinweis anzeigen, nie stillschweigend Programmcode nachladen
- [ ] **S** Deinstallation entfernt Aufgabe, Zustand und auf Nachfrage die Logs
- [ ] **K** Portable Variante ohne Installation und ohne Reminder, für den USB-Stick

## Sonderfälle

Die meisten Fehler eines solchen Tools entstehen in Sonderfällen, deshalb bekommt jeder einen festen Umgang: erkennen, in einem Satz sagen, was los ist, und im Zweifel nur lesen statt ändern.

- [ ] **M** Verwalteter PC (Firma, Schule, Behörde): erkennen (Domäne, Entra-ID-Anmeldung per `dsregcmd /status`, MDM-Registrierung) und dann nur lesen, weil die IT Updates und Upgrades steuert
- [ ] **M** Virtuelle Maschine erkennen (Hyper-V, VMware, VirtualBox) und Grafik- und Hardwareprüfung anpassen, statt Fehler zu melden
- [ ] **M** Zwei Grafikkarten im Laptop: beide anzeigen und die integrierte nie als Treiber fehlt melden
- [ ] **M** Windows Server, LTSC und S-Modus erkennen: Upgrade-Modus sperren und die winget-Teile nur als nicht verfügbar melden
- [ ] **M** ARM-PCs erkennen (`PROCESSOR_ARCHITECTURE`), denn die CPU-Listen für Intel und AMD gelten dort nicht
- [ ] **M** Offline oder hinter Proxy: Online-Prüfungen werden Unbekannt, alle lokalen Prüfungen laufen weiter
- [ ] **M** Immer als 64-Bit-Prozess laufen, sonst leitet Windows Registry-Zugriffe und den Ordner System32 um und Prüfungen lesen falsche Werte
- [ ] **M** Das Systemlaufwerk nie als C: annehmen, sondern aus `%SystemDrive%` lesen
- [ ] **S** Dual-Boot erkennen (`bcdedit /enum`) und vor dem Upgrade warnen, weil der Bootloader betroffen sein kann
- [ ] **S** Benutzernamen und Pfade mit Leerzeichen, Umlauten oder Sonderzeichen testen (zum Beispiel Zoë Müller), jeder Pfad wird korrekt in Anführungszeichen gesetzt
- [ ] **S** Wenig Speicherplatz (kleine SSD): Hinweis, wie sich Platz schaffen lässt, ohne selbst etwas zu löschen
- [ ] **K** Insider- und Vorschau-Builds erkennen und nicht wie eine stabile Version behandeln
- [ ] **K** Mehrere Benutzerkonten: Reminder und Berichte pro Benutzer, nicht für andere sichtbar

## Quellen

Geprüft am 3. November 2026, alle Seiten wurden geöffnet.

- [Microsoft Learn: Update Secure Boot Certificates for Windows Devices](https://learn.microsoft.com/en-us/troubleshoot/windows-client/windows-security/update-secure-boot-certificates): Folgen eines fehlenden Updates, Firmware zuerst, BitLocker-Abfragen
- [Microsoft Learn: Use Windows Setup compatibility scan logs](https://learn.microsoft.com/en-us/troubleshoot/windows-client/setup-upgrade-and-drivers/use-windows-setup-compatibility-scan-logs-to-identify-blocking-issues): Rückgabecodes 0xC1900210 und 0xC1900208, Logorte, SetupDiag
- [endoflife.ai: Windows 10 ESU moved to October 12, 2027](https://endoflife.ai/article-windows-10-consumer-esu-drift): Support-Enden von Windows 10 und Windows 11 24H2, Stand 9. September 2026
- [Richard M. Hicks: Windows Secure Boot UEFI Certificates Expiring June 2026](https://directaccess.richardhicks.com/2025/12/04/windows-secure-boot-uefi-certificates-expiring-june-2026/): Registry-Wert UEFICA2023Status, AvailableUpdates 0x5944, Aufgabe Secure-Boot-Update, Ablaufmonate der Zertifikate
- [winget-cli auf GitHub](https://github.com/microsoft/winget-cli): Standardquellen winget und msstore, PowerShell-Modul, Telemetrie des winget-Clients

Befehle, Rückgabecodes und Umrechnungsregeln ohne Link (etwa die Treiberversionen von NVIDIA und Intel oder die Optionen von DISM und SFC) stammen aus Fachwissen und gehören vor dem Einbau einmal in einer VM geprüft.

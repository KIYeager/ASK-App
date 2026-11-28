# Docs per Link teilen: Einrichtung bei Microsoft

Damit Lernende ein Doc mit einem Link teilen können (in Teams schicken, die anderen fügen ihn in ASK ein und schreiben mit), meldet sich ASK bei Microsoft an und legt das Doc im OneDrive der Person ab (ENTSCHEIDE E83). Dafür braucht ASK einmal eine **App-Registrierung** in Microsoft Entra ID der TF Bern. Ohne sie zeigt ASK das Teilen per Link gar nicht an; geteilt wird dann über einen Ordner, den alle haben (E82, E89).

## Was die App-Registrierung erlaubt

- ASK meldet die Person mit ihrem Schulkonto an, im Browser, wie jede Desktop-App von Microsoft. ASK sieht kein Passwort.
- Berechtigungen, alle **delegiert** (nur im Namen der angemeldeten Person, nur auf das, worauf sie selbst Zugriff hat):
  - `Files.ReadWrite.All`: Doc im eigenen OneDrive unter «ASK Docs» anlegen und Ordner öffnen, die andere mit einem Link geteilt haben
  - `User.Read`: Name der Person anzeigen
  - `offline_access`: angemeldet bleiben, ohne jedes Mal neu anzumelden
- Der Link ist ein normaler Link von OneDrive «Personen in der TF Bern können bearbeiten». Er gilt nur innerhalb der Schule.
- Es gibt kein Client-Geheimnis. Die Kennung der App ist nicht geheim und steht in der App selbst.

## Anleitung (für die Informatik, etwa 5 Minuten)

1. https://entra.microsoft.com öffnen, mit einem Konto, das App-Registrierungen anlegen darf.
2. **Identität → Anwendungen → App-Registrierungen → Neue Registrierung**
   - Name: `ASK Desktop`
   - Unterstützte Kontotypen: **Nur Konten in diesem Organisationsverzeichnis** (ein Mandant)
   - Umleitungs-URI: Plattform **Öffentlicher Client/nativ (mobil und Desktop)**, URI `http://localhost`
   - **Registrieren**
3. **API-Berechtigungen → Berechtigung hinzufügen → Microsoft Graph → Delegierte Berechtigungen**: `Files.ReadWrite.All`, `User.Read`, `offline_access` hinzufügen. Danach **Administratorzustimmung für TF Bern erteilen**, damit die Lernenden nicht selbst zustimmen müssen (oder es nicht dürfen).
4. Auf der Seite **Übersicht** zwei Werte kopieren: **Anwendungs-ID (Client)** und **Verzeichnis-ID (Mandant)**.
5. In SharePoint (Admin Center → Richtlinien → Freigabe) müssen Links für **Personen in der Organisation** erlaubt sein. Das ist der Standard.

## In ASK eintragen

In `resources/app-config.json`:

```json
"microsoft": { "clientId": "<Anwendungs-ID>", "tenant": "<Verzeichnis-ID>" }
```

Per Pull Request mit Label `beta` oder `release`. Mit der nächsten Version können alle per Link teilen.

## Nachricht an die Informatik (zum Weiterleiten)

> Hallo
>
> Für ASK Desktop (die App für ASK, die Lernende an der TF Bern nutzen) möchten wir, dass Lernende Dokumente per Link gemeinsam bearbeiten können, so wie in Teams. Die App legt das Dokument dazu im OneDrive der Person ab und erstellt einen Link «Personen in der TF Bern können bearbeiten».
>
> Dafür bräuchten wir eine App-Registrierung in Entra ID: ein Mandant, öffentlicher Client mit Umleitungs-URI `http://localhost`, delegierte Berechtigungen `Files.ReadWrite.All`, `User.Read` und `offline_access`, mit Administratorzustimmung. Es gibt kein Client-Geheimnis; die App greift nur im Namen der angemeldeten Person auf deren Dateien zu.
>
> Könnt ihr sie anlegen und uns die Anwendungs-ID und die Verzeichnis-ID schicken? Die genaue Anleitung steht unten.
>
> Danke!

## Grenzen

- Wie schnell Änderungen ankommen: ASK fragt OneDrive alle 1,5 Sekunden nach Neuem. Meist kommen Änderungen nach 2 bis 3 Sekunden an.
- Bei Microsoft muss jede Person einmal angemeldet sein. Läuft die Anmeldung ab (je nach Einstellung der Schule nach Wochen), fragt ASK neu.
- Nicht mit dem echten Microsoft 365 der Schule getestet, nur gegen einen Nachbau der Schnittstelle (`docsync::tests::zwei_laptops_per_link_ueber_onedrive`).

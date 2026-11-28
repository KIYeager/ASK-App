// Prüft Word-Dateien mit dem Validator des Open XML SDK von Microsoft: gültig für Word 2007, 2016 und Microsoft 365.
// Aufruf: dotnet run --project tests/docx-pruefen -- a.docx b.docx …  (Exitcode 1 bei Fehlern)
// Die Dateien schreibt der Test docx::tests::beispiel_mit_allem (ENTSCHEIDE E82).
using DocumentFormat.OpenXml;
using DocumentFormat.OpenXml.Packaging;
using DocumentFormat.OpenXml.Validation;

if (args.Length == 0) { Console.Error.WriteLine("Keine Dateien angegeben."); return 2; }
int bad = 0;
foreach (var path in args) {
    using var doc = WordprocessingDocument.Open(path, false);
    foreach (var version in new[] { FileFormatVersions.Office2007, FileFormatVersions.Office2016, FileFormatVersions.Microsoft365 }) {
        var errors = new OpenXmlValidator(version).Validate(doc).ToList();
        if (errors.Count == 0) continue;
        Console.WriteLine($"{Path.GetFileName(path)} ({version}): {errors.Count} Fehler");
        foreach (var e in errors.Take(20)) Console.WriteLine($"  {e.Part?.Uri} {e.Path?.XPath}: {e.Description}");
        bad += errors.Count;
    }
}
Console.WriteLine(bad == 0 ? $"{args.Length} Word-Dateien gültig." : $"{bad} Fehler.");
return bad == 0 ? 0 : 1;

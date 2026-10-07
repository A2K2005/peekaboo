#requires -Version 5.1
[CmdletBinding()]
param([string]$OutputDirectory = (Join-Path $PSScriptRoot '../fixtures'))
$ErrorActionPreference = 'Stop'
$output = [IO.Path]::GetFullPath($OutputDirectory)
[IO.Directory]::CreateDirectory($output) | Out-Null
Add-Type -AssemblyName System.Drawing
if (-not ('PreviewFixtures' -as [type])) {
Add-Type -TypeDefinition @"
using System;
using System.IO;
using System.Text;
using System.Collections.Generic;
public static class PreviewFixtures {
  static void Write(FileStream f, string s) { var b=Encoding.ASCII.GetBytes(s); f.Write(b,0,b.Length); }
  public static void Pdf(string path, int pages, int padding) {
    using(var f=new FileStream(path,FileMode.Create,FileAccess.ReadWrite)) {
      var offsets=new List<long>(); offsets.Add(0);
      Write(f,"%PDF-1.4\n");
      Action<int,string> obj=(id,body)=>{ if(id!=offsets.Count) throw new Exception("Object order"); offsets.Add(f.Position); Write(f,id+" 0 obj\n"+body+"\nendobj\n"); };
      obj(1,"<< /Type /Catalog /Pages 2 0 R >>");
      var kids=new StringBuilder(); for(int p=0;p<pages;p++) kids.Append((4+p*2)+" 0 R ");
      obj(2,"<< /Type /Pages /Count "+pages+" /Kids [ "+kids+" ] >>");
      obj(3,"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>");
      byte[] spaces=new byte[8192]; for(int j=0;j<spaces.Length;j++) spaces[j]=32;
      for(int p=0;p<pages;p++) {
        int id=4+p*2;
        obj(id,"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 3 0 R >> >> /Contents "+(id+1)+" 0 R >>");
        string content="BT /F1 24 Tf 50 700 Td (Synthetic page "+(p+1)+" of "+pages+") Tj ET\n";
        offsets.Add(f.Position);
        Write(f,(id+1)+" 0 obj\n<< /Length "+(Encoding.ASCII.GetByteCount(content)+padding)+" >>\nstream\n"+content);
        int left=padding; while(left>0) { int n=Math.Min(left,spaces.Length); f.Write(spaces,0,n); left-=n; }
        Write(f,"\nendstream\nendobj\n");
      }
      long xref=f.Position;
      Write(f,"xref\n0 "+offsets.Count+"\n0000000000 65535 f \n");
      for(int i=1;i<offsets.Count;i++) Write(f,offsets[i].ToString("D10")+" 00000 n \n");
      Write(f,"trailer\n<< /Size "+offsets.Count+" /Root 1 0 R >>\nstartxref\n"+xref+"\n%%EOF\n");
      f.Flush();
      // Verify every generated xref points at the corresponding object header.
      for(int i=1;i<offsets.Count;i++) { f.Position=offsets[i]; byte[] b=new byte[(i+" 0 obj").Length]; if(f.Read(b,0,b.Length)!=b.Length || Encoding.ASCII.GetString(b)!=i+" 0 obj") throw new Exception("Invalid xref"); }
    }
  }
}
"@
}
[PreviewFixtures]::Pdf((Join-Path $output '20-pages.pdf'),20,0)
[PreviewFixtures]::Pdf((Join-Path $output '500-pages-50mb.pdf'),500,100000)
foreach ($spec in @(@('image-small.png',640,480),@('image-24mp.jpg',6000,4000))) {
    $bitmap = [Drawing.Bitmap]::new([int]$spec[1],[int]$spec[2])
    $graphics = [Drawing.Graphics]::FromImage($bitmap)
    try {
        $graphics.Clear([Drawing.Color]::CornflowerBlue)
        $graphics.FillRectangle([Drawing.Brushes]::Orange,0,0,[int]($spec[1]/2),[int]($spec[2]/2))
        $format = if ($spec[0].EndsWith('.jpg')) { [Drawing.Imaging.ImageFormat]::Jpeg } else { [Drawing.Imaging.ImageFormat]::Png }
        $bitmap.Save((Join-Path $output $spec[0]),$format)
    } finally { $graphics.Dispose(); $bitmap.Dispose() }
}
$bitmap = [Drawing.Bitmap]::new(1200,260)
$graphics = [Drawing.Graphics]::FromImage($bitmap)
$font = [Drawing.Font]::new('Arial',40,[Drawing.FontStyle]::Regular,[Drawing.GraphicsUnit]::Pixel)
try {
    $graphics.Clear([Drawing.Color]::White)
    $graphics.DrawString('Preview for Windows',$font,[Drawing.Brushes]::Black,[single]30,[single]40)
    $graphics.DrawString('Offline text recognition 12345',$font,[Drawing.Brushes]::Black,[single]30,[single]120)
    $bitmap.Save((Join-Path $output 'ocr-text.png'),[Drawing.Imaging.ImageFormat]::Png)
} finally { $font.Dispose(); $graphics.Dispose(); $bitmap.Dispose() }
[IO.File]::WriteAllText((Join-Path $output 'corrupt.pdf'),'%PDF-1.7 broken xref')
[IO.File]::WriteAllText((Join-Path $output 'corrupt.jpg'),'not a JPEG')
$objects = @(
    '<< /Type /Catalog /Pages 2 0 R /AcroForm 6 0 R >>',
    '<< /Type /Pages /Count 1 /Kids [3 0 R] >>',
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Annots [5 0 R] /Resources << /Font << /Helv 4 0 R >> >> >>',
    '<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>',
    '<< /Type /Annot /Subtype /Widget /FT /Tx /T (Customer name) /V (Original value) /Rect [50 650 350 700] /P 3 0 R /DA (/Helv 24 Tf 0 g) /F 4 >>',
    '<< /Fields [5 0 R] /DR << /Font << /Helv 4 0 R >> >> /DA (/Helv 24 Tf 0 g) /NeedAppearances true >>'
)
$stream = [IO.File]::Create((Join-Path $output 'acroform-text.pdf'))
try {
    $offsets = [Collections.Generic.List[long]]::new()
    $bytes = [Text.Encoding]::ASCII.GetBytes("%PDF-1.4`n"); $stream.Write($bytes, 0, $bytes.Length)
    for ($i=0; $i -lt $objects.Count; $i++) {
        $offsets.Add($stream.Position)
        $bytes = [Text.Encoding]::ASCII.GetBytes("$($i+1) 0 obj`n$($objects[$i])`nendobj`n"); $stream.Write($bytes, 0, $bytes.Length)
    }
    $xref = $stream.Position
    $tail = "xref`n0 $($objects.Count+1)`n0000000000 65535 f `n"
    foreach ($offset in $offsets) { $tail += $offset.ToString('D10') + " 00000 n `n" }
    $tail += "trailer`n<< /Size $($objects.Count+1) /Root 1 0 R >>`nstartxref`n$xref`n%%EOF`n"
    $bytes = [Text.Encoding]::ASCII.GetBytes($tail); $stream.Write($bytes, 0, $bytes.Length)
} finally { $stream.Dispose() }
$manifest = @(Get-ChildItem -LiteralPath $output -File | Where-Object Name -ne 'manifest.json' | ForEach-Object {
    [ordered]@{name=$_.Name; bytes=$_.Length; sha256=(Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash}
})
$manifest | ConvertTo-Json -Depth 3 | Set-Content -LiteralPath (Join-Path $output 'manifest.json') -Encoding utf8
Write-Output "Generated and xref-checked fixtures in $output. Synthetic PDFs use whitespace padding, not representative complex content."

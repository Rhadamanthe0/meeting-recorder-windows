<?xml version="1.0" encoding="UTF-8"?>
<!-- Transformée heat : exclut l'exe principal de la récolte car Product.wxs
     le déclare déjà (avec ses raccourcis). Tout le reste du stage
     (DLLs ORT/MinGW/GTK, ffmpeg, data GTK) est récolté tel quel. -->
<xsl:stylesheet version="1.0"
    xmlns:xsl="http://www.w3.org/1999/XSL/Transform"
    xmlns:wix="http://schemas.microsoft.com/wix/2006/wi">
  <xsl:output method="xml" indent="yes" />
  <xsl:key name="exe-search"
           match="wix:Component[contains(wix:File/@Source, 'meeting-recorder-windows.exe')]"
           use="@Id" />
  <xsl:template match="@*|node()">
    <xsl:copy>
      <xsl:apply-templates select="@*|node()" />
    </xsl:copy>
  </xsl:template>
  <xsl:template match="wix:Component[contains(wix:File/@Source, 'meeting-recorder-windows.exe')]" />
  <xsl:template match="wix:ComponentRef[key('exe-search', @Id)]" />
</xsl:stylesheet>

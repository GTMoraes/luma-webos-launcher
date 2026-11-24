# Luma no Android TV (Chromecast com Google TV)

O mesmo Luma da TV LG, compilado para Android TV. O código Rust é o mesmo; o que muda fica em
`src/android.rs` e nesta pasta:

- `app/src/main/java/com/frx9/luma/LumaActivity.java`: a tela do Android. Ela prepara as pastas,
  copia os arquivos do Luma na primeira abertura de cada versão e responde à lista de apps, aos
  banners e ao abrir apps.
- `build-native.sh`: compila a SDL2 (versão fixa, conferida por SHA-256), o `libluma.so` e junta
  os arquivos gerados pelo `pnpm assets`.
- `.github/workflows/android.yml`: monta o `.apk` assinado no GitHub Actions.

No Android não existem: Moonlight, Jellyfin, Plex, player, música, cripto, entradas HDMI e papel
de parede em vídeo. O papel de parede é escolhido entre as fotos em Ajustes › Wallpaper. Voltar
na tela inicial abre o Mystify (`com.frx9.mystify`), como o Google TV faz com o protetor dele.

## Chave de assinatura

O workflow usa os mesmos quatro segredos do Mystify (mesma chave, assim as atualizações instalam
por cima): `MYS_KEYSTORE_B64`, `MYS_KEYSTORE_PASS`, `MYS_KEY_ALIAS` e `MYS_KEY_PASS`, em Settings ›
Secrets and variables › Actions deste repositório. Sem eles o `.apk` sai com chave de debug, que
muda a cada execução.

## Instalar e atualizar

Baixe o artefato `luma-android-N` da execução do workflow Android e:

```powershell
& $A install -r luma-android-2.2.160-N.apk
& $A shell am start -n com.frx9.luma/.LumaActivity
```

## Arquivos no aparelho

Na pasta `/sdcard/Android/data/com.frx9.luma/files/`:

- `preferences.json`: as preferências, no mesmo formato da TV.
- `luma.log`: o log (o anterior vira `luma.log.1` depois de 1 MiB).
- `ha-auth.json`: se colocado aqui, a credencial do Home Assistant é importada na próxima abertura
  e o arquivo é apagado desta pasta; ela fica na área privada do app.

```powershell
& $A shell am force-stop com.frx9.luma
& $A push preferences.json /sdcard/Android/data/com.frx9.luma/files/preferences.json
& $A shell cat /sdcard/Android/data/com.frx9.luma/files/luma.log
```

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

O botão de ajustes no canto superior direito abre as configurações rápidas do Luma: rede, tela e
som, protetor de tela, apps, data e hora, todas as configurações e as notificações. As
notificações aparecem depois que o Luma recebe o Acesso a notificações (a própria linha do painel
abre essa página). Segurar OK num app da lista de Apps oferece Desinstalar, que o sistema confirma
antes de remover.

## Luma como tela inicial

O Luma se declara como tela inicial. Para o botão Home abrir o Luma sem serviço de acessibilidade,
o launcher do Google TV e o assistente de configuração precisam ficar desativados:

```powershell
& $A shell pm disable-user --user 0 com.google.android.apps.tv.launcherx
& $A shell pm disable-user --user 0 com.google.android.tungsten.setupwraith
& $A shell cmd package set-home-activity --user 0 com.frx9.luma/.LumaActivity
```

Para voltar ao launcher do Google:

```powershell
& $A shell pm enable --user 0 com.google.android.apps.tv.launcherx
& $A shell pm enable --user 0 com.google.android.tungsten.setupwraith
& $A shell cmd package set-home-activity --user 0 com.google.android.apps.tv.launcherx/.home.HomeActivity
```

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

As preferências ficam na área privada do app (`/data/data/com.frx9.luma/files/`), que está
sempre pronta quando o Luma abre. A pasta compartilhada pode ainda não estar montada logo
depois de ligar o Chromecast: em 24/11/2026 o Luma abriu com o padrão depois de um boot frio,
quando as preferências moravam lá. Na primeira abertura desta versão com a pasta compartilhada
disponível, o `preferences.json` dela (com o cache do clima e os ícones) é movido para a área
privada, por cima de qualquer arquivo que já estivesse lá.

Na pasta `/sdcard/Android/data/com.frx9.luma/files/`:

- `preferences.json`: cópia das preferências, gravada a cada mudança (backup; o Luma não lê).
- `preferences-importar.json`: se colocado aqui, substitui as preferências na próxima abertura
  e é apagado. Arquivo que o Luma não consegue ler fica, e o motivo vai para o log.
- `luma.log`: o log (o anterior vira `luma.log.1` depois de 1 MiB). Sem a pasta pronta no boot,
  o log daquela abertura vai para a área privada.
- `ha-auth.json`: se colocado aqui, a credencial do Home Assistant é importada na próxima abertura
  e o arquivo é apagado desta pasta; ela fica na área privada do app.

```powershell
& $A pull /sdcard/Android/data/com.frx9.luma/files/preferences.json
& $A push preferences.json /sdcard/Android/data/com.frx9.luma/files/preferences-importar.json
& $A shell am force-stop com.frx9.luma
& $A shell cat /sdcard/Android/data/com.frx9.luma/files/luma.log
```

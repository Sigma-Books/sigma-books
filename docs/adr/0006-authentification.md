# 0006 — Authentification : passkeys, TOTP en secours, jamais de SMS

- **Statut :** acceptée — **Date :** 2026-11-12

## Décision
- **Passkeys (WebAuthn)** comme méthode principale ; **TOTP** en secours ; **pas de SMS** (SIM swapping).
- MFA obligatoire pour tous les rôles cabinet dès la création du compte.
- Sessions par cookie `HttpOnly`, `Secure`, `SameSite=Strict`, durée courte, rotation à chaque élévation.
- Ré-authentification pour les actions sensibles (validation par lot, clôture, déverrouillage, export complet, droits).

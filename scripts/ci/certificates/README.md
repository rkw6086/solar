# Apple signing intermediate

`DeveloperIDG2CA.pem` is Apple's public Developer ID G2 intermediate certificate,
downloaded from <https://www.apple.com/certificateauthority/DeveloperIDG2CA.cer>
and converted from DER to PEM. It expires September 17, 2031.

Its certificate SHA-256 fingerprint is
`f16cd3c54c7f83cea4bf1a3e6a0819c8aaa8e4a1528fd144715f350643d2df3a`.

The release script adds it only to the temporary signing keychain. It does not
install a new trust anchor or change the Mac's certificate trust settings.
The signing chain must verify against the Mac's existing trusted roots.

`AppleRootCA.pem` is the public certificate from
<https://www.apple.com/appleca/AppleIncRootCertificate.cer>, with SHA-256
`b0b1730ecbc7ff4505142c49f1295e6eda6bcaed7e2c68c5be91b5a11001f024`.
The diagnostic check can add this certificate to its temporary keychain to
test complete-chain discovery; it never assigns trust to that certificate.

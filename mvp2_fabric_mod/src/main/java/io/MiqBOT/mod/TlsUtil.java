package io.MiqBOT.mod;

import javax.net.ssl.KeyManagerFactory;
import javax.net.ssl.SSLContext;
import javax.net.ssl.TrustManagerFactory;
import java.io.FileInputStream;
import java.security.KeyStore;

public final class TlsUtil {
    private TlsUtil() {}

    public static SSLContext buildMtlsContext(
            String clientP12Path,
            String clientP12Password,
            String trustP12Path,
            String trustP12Password
    ) throws Exception {
        KeyStore clientKs = KeyStore.getInstance("PKCS12");
        try (FileInputStream in = new FileInputStream(clientP12Path)) {
            clientKs.load(in, clientP12Password.toCharArray());
        }

        KeyManagerFactory kmf = KeyManagerFactory.getInstance(KeyManagerFactory.getDefaultAlgorithm());
        kmf.init(clientKs, clientP12Password.toCharArray());

        KeyStore trustKs = KeyStore.getInstance("PKCS12");
        try (FileInputStream in = new FileInputStream(trustP12Path)) {
            trustKs.load(in, trustP12Password.toCharArray());
        }

        TrustManagerFactory tmf = TrustManagerFactory.getInstance(TrustManagerFactory.getDefaultAlgorithm());
        tmf.init(trustKs);

        SSLContext ctx = SSLContext.getInstance("TLS");
        ctx.init(kmf.getKeyManagers(), tmf.getTrustManagers(), null);
        return ctx;
    }
}

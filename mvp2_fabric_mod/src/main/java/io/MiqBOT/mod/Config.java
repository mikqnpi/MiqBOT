package io.MiqBOT.mod;

import java.io.FileInputStream;
import java.io.IOException;
import java.nio.file.Path;
import java.util.Properties;

public final class Config {
    public final String bridgeUrl;
    public final String agentId;

    public final String clientP12Path;
    public final String clientP12Password;

    public final String trustP12Path;
    public final String trustP12Password;

    private Config(
            String bridgeUrl,
            String agentId,
            String clientP12Path,
            String clientP12Password,
            String trustP12Path,
            String trustP12Password
    ) {
        this.bridgeUrl = bridgeUrl;
        this.agentId = agentId;
        this.clientP12Path = clientP12Path;
        this.clientP12Password = clientP12Password;
        this.trustP12Path = trustP12Path;
        this.trustP12Password = trustP12Password;
    }

    public static Config load(Path configDir) throws IOException {
        Path p = configDir.resolve("MiqBOT-client.properties");

        if (!p.toFile().exists()) {
            Properties defaults = new Properties();
            defaults.setProperty("bridge_url", "wss://127.0.0.1:40100");
            defaults.setProperty("agent_id", "gamepc");
            defaults.setProperty("client_p12_path", "C:/MiqBOT/certs/client.p12");
            defaults.setProperty("client_p12_password", "changeit");
            defaults.setProperty("trust_p12_path", "C:/MiqBOT/certs/ca.p12");
            defaults.setProperty("trust_p12_password", "changeit");
            try (var out = java.nio.file.Files.newOutputStream(p)) {
                defaults.store(out, "MiqBOT client config (ASCII paths preferred)");
            }
        }

        Properties props = new Properties();
        try (FileInputStream in = new FileInputStream(p.toFile())) {
            props.load(in);
        }

        return new Config(
                props.getProperty("bridge_url", "wss://127.0.0.1:40100").trim(),
                props.getProperty("agent_id", "gamepc").trim(),
                props.getProperty("client_p12_path", "").trim(),
                props.getProperty("client_p12_password", "").trim(),
                props.getProperty("trust_p12_path", "").trim(),
                props.getProperty("trust_p12_password", "").trim()
        );
    }
}

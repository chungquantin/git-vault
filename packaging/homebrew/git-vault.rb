class GitVault < Formula
  desc "Manage multiple git identities and credentials per command, aws-vault style"
  homepage "https://github.com/chungquantin/git-vault"
  head "https://github.com/chungquantin/git-vault.git", branch: "main"

  depends_on "python"

  def install
    bin.install "git-vault"
  end

  def caveats
    <<~EOS
      Enable `git add vault` and `git <cmd> --vault <name>` by adding to ~/.zshrc:
        eval "$(git-vault shell-init)"
    EOS
  end

  test do
    ENV["GIT_VAULT_HOME"] = testpath/"vaults"
    assert_match "git-vault", shell_output("#{bin}/git-vault --version")
    system bin/"git-vault", "add", "--name", "t", "--user-name", "T", "--email", "t@example.com", "--no-token"
    assert_match "t@example.com", shell_output("#{bin}/git-vault list")
  end
end

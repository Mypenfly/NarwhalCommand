// Dart Authentication Service
// Handles OAuth2 login, token refresh, and session validation.

class AuthResult {
  final String token;
  final String refreshToken;
  final DateTime expiresAt;

  AuthResult({
    required this.token,
    required this.refreshToken,
    required this.expiresAt,
  });

  bool get isExpired => DateTime.now().isAfter(expiresAt);
}

class AuthService {
  final String baseUrl;
  final HttpClient _client;

  AuthService({required this.baseUrl})
      : _client = HttpClient();

  Future<AuthResult> login(String email, String password) async {
    final response = await _client.post(
      Uri.parse('$baseUrl/auth/login'),
      body: {'email': email, 'password': password},
    );

    if (response.statusCode != 200) {
      throw AuthException('Login failed: ${response.body}');
    }

    final data = jsonDecode(response.body);
    return AuthResult(
      token: data['access_token'],
      refreshToken: data['refresh_token'],
      expiresAt: DateTime.now().add(
        Duration(seconds: data['expires_in']),
      ),
    );
  }

  Future<AuthResult> refresh(String refreshToken) async {
    final response = await _client.post(
      Uri.parse('$baseUrl/auth/refresh'),
      body: {'refresh_token': refreshToken},
    );

    if (response.statusCode != 200) {
      throw AuthException('Refresh failed');
    }

    final data = jsonDecode(response.body);
    return AuthResult(
      token: data['access_token'],
      refreshToken: data['refresh_token'],
      expiresAt: DateTime.now().add(
        Duration(seconds: data['expires_in']),
      ),
    );
  }

  Future<bool> validateSession(String token) async {
    final response = await _client.get(
      Uri.parse('$baseUrl/auth/validate'),
      headers: {'Authorization': 'Bearer $token'},
    );
    return response.statusCode == 200;
  }

  void dispose() {
    _client.close();
  }
}

class AuthException implements Exception {
  final String message;
  AuthException(this.message);
}
